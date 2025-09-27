use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use dashmap::DashMap;
use futures_util::{stream::StreamExt, SinkExt};
use tokio::sync::mpsc;
use tracing::info;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use poker_eden_core::{ClientMessage, GamePhase, GameState, Player, PlayerId, PlayerSecret, PlayerState, RoomId, ServerMessage};

// 服务器全局状态，使用 Arc<Mutex<...>> 实现线程安全共享
struct AppState {
    rooms: DashMap<RoomId, Room>,
}

// 单个房间的状态
struct Room {
    game_state: GameState,
    host_id: PlayerId,
    // 将 PlayerId 映射到具体的网络连接
    players: HashMap<PlayerId, PlayerConnection>,
}

// 玩家的网络连接信息
struct PlayerConnection {
    secret: PlayerSecret,
    // 用于向该玩家的 WebSocket 任务发送消息的通道
    sender: mpsc::Sender<ServerMessage>,
}

type SharedState = Arc<AppState>;

#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    // 初始化订阅者
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(filter).finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let state = SharedState::new(AppState {
        rooms: DashMap::new(),
    });

    let app = Router::new()
        .route("/ws", get(websocket_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 25917));
    info!("服务器正在监听 {}", addr);
    axum::serve(tokio::net::TcpListener::bind(addr).await.unwrap(), app)
        .await
        .unwrap();
}

/// 处理 WebSocket 连接请求
async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

/// 处理单个 WebSocket 连接的生命周期
async fn handle_socket(socket: WebSocket, state: SharedState) {
    let (mut sender, mut receiver) = socket.split();

    // 创建一个 MPSC 通道，用于从其他任务接收要发送的消息
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(32);

    // 启动一个新任务，专门负责将 MPSC 通道中的消息发送到 WebSocket
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let payload = serde_json::to_string(&msg).unwrap();
            if sender.send(Message::Text(payload.into())).await.is_err() {
                // 发送失败，说明客户端已断开，退出任务
                break;
            }
        }
    });

    // 当前连接的上下文信息，在认证成功后填充
    let mut player_context: Option<(RoomId, PlayerId)> = None;

    // 主循环，处理从客户端接收到的消息
    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            match serde_json::from_str::<ClientMessage>(&text) {
                Ok(client_msg) => {
                    handle_client_message(
                        client_msg,
                        state.clone(),
                        &tx,
                        &mut player_context,
                    ).await;
                }
                Err(e) => {
                    tracing::warn!("解析消息失败: {}", e);
                }
            }
        }
    }

    // 客户端断开连接，执行清理工作
    if let Some((room_id, player_id)) = player_context {
        handle_disconnect(state, room_id, player_id).await;
    }
}

/// 核心消息处理逻辑
async fn handle_client_message(
    msg: ClientMessage,
    state: SharedState,
    tx: &mpsc::Sender<ServerMessage>,
    context: &mut Option<(RoomId, PlayerId)>,
) {
    match msg {
        ClientMessage::CreateRoom { nickname } => {
            if context.is_some() {
                let _ = tx.send(ServerMessage::Error { message: "你已经在一个房间里了".to_string() }).await;
                return;
            }

            let room_id = Uuid::new_v4();
            let player_id = Uuid::new_v4();
            let player_secret = Uuid::new_v4();

            let mut game_state = GameState::default();
            game_state.room_id = room_id;

            let player = Player {
                id: player_id,
                nickname,
                stack: 0,
                wins: 0,
                losses: 0,
                state: PlayerState::SittingOut,
                seat_id: None,
            };
            game_state.players.insert(player_id, player.clone());
            let gs_for_client = game_state.for_client(&player_id);

            let mut room = Room {
                game_state,
                host_id: player_id,
                players: HashMap::new(),
            };
            room.players.insert(player_id, PlayerConnection {
                secret: player_secret,
                sender: tx.clone(),
            });

            state.rooms.insert(room_id, room);

            *context = Some((room_id, player_id));

            let _ = tx.send(ServerMessage::RoomJoined {
                your_id: player_id,
                your_secret: player_secret,
                game_state: gs_for_client,
                host_id: player_id,
            }).await;
            info!("玩家 {} 创建了新房间 {}", player_id, room_id);
        }
        ClientMessage::JoinRoom { room_id, nickname } => {
            if context.is_some() {
                let _ = tx.send(ServerMessage::Error { message: "你已经在一个房间里了".to_string() }).await;
                return;
            }

            let player_id = Uuid::new_v4();
            let player_secret = Uuid::new_v4();

            let targets;
            let join_broadcast_msg;
            let join_msg;
            {
                let mut room = match state.rooms.get_mut(&room_id) {
                    Some(r) => r,
                    None => {
                        let _ = tx.send(ServerMessage::Error { message: "房间不存在".to_string() }).await;
                        return;
                    }
                };

                *context = Some((room_id, player_id));

                let player = Player {
                    id: player_id,
                    nickname,
                    stack: 0,
                    wins: 0,
                    losses: 0,
                    state: PlayerState::SittingOut,
                    seat_id: None,
                };

                room.game_state.players.insert(player_id, player.clone());
                room.players.insert(player_id, PlayerConnection {
                    secret: player_secret,
                    sender: tx.clone(),
                });

                let gs_for_client = room.game_state.for_client(&player_id);

                targets = create_msg_targets(&room.players);
                join_broadcast_msg = ServerMessage::PlayerJoined { player: player.clone() };
                join_msg = ServerMessage::RoomJoined {
                    your_id: player_id,
                    your_secret: player_secret,
                    game_state: gs_for_client,
                    host_id: room.host_id,
                };
            }

            broadcast(&targets, &join_broadcast_msg, Some(player_id)).await;
            let _ = tx.send(join_msg).await;
            info!("玩家 {} 加入了房间 {}", player_id, room_id);
        }
        // ... 其他需要认证后才能执行的消息
        _ => {
            if let Some((room_id, player_id)) = context {
                let targets;
                let mut only_messages = vec![];
                let broadcast_messages = {
                    let mut room = match state.rooms.get_mut(&room_id) {
                        Some(r) => r,
                        None => {
                            let _ = tx.send(ServerMessage::Error { message: "房间不存在".to_string() }).await;
                            return;
                        }
                    };

                    targets = create_msg_targets(&room.players);

                    // 游戏逻辑处理
                    match msg {
                        ClientMessage::StartHand => {
                            if *player_id != room.host_id {
                                vec![ServerMessage::Error { message: "只有房主可以开始游戏".to_string() }]
                            } else {
                                room.game_state.seated_players.rotate_left(1);
                                room.game_state.start_new_hand()
                            }
                        }
                        ClientMessage::RequestSeat { seat_id, stack } => {
                            if !(room.game_state.phase == GamePhase::WaitingForPlayers || room.game_state.phase == GamePhase::Showdown) {
                                only_messages.push(ServerMessage::Error { message: "入座失败：请在等待阶段入座".to_string() });
                                vec![]
                            } else if seat_id >= room.game_state.seats {
                                only_messages.push(ServerMessage::Error { message: "入座失败：座位号超出最大座位数".to_string() });
                                vec![]
                            } else if room.game_state.players.values().any(|p| p.seat_id == Some(seat_id) && p.id != *player_id) {
                                only_messages.push(ServerMessage::Error { message: "入座失败：该位置已有玩家入座".to_string() });
                                vec![]
                            } else {
                                if let Some(idx) = room.game_state.seated_players.iter().position(|p| *p == *player_id) {
                                    room.game_state.seated_players.remove(idx);
                                }
                                let p = {
                                    let p = room.game_state.players.get_mut(&player_id).unwrap();
                                    p.stack = stack;
                                    p.seat_id = Some(seat_id);
                                    p.state = PlayerState::Waiting;
                                    p.clone()
                                };
                                let sid = room.game_state.find_insertion_index(seat_id);
                                room.game_state.seated_players.insert(sid, p.id);

                                vec![ServerMessage::PlayerUpdated { player: p }]
                            }
                        }
                        ClientMessage::PerformAction(action) => {
                            let mut msg = room.game_state.handle_player_action(*player_id, action);
                            let rs = room.game_state.tick();
                            if rs.0 {
                                msg.extend(rs.1);
                            }
                            msg
                        }
                        ClientMessage::GetMyHand => {
                            if room.game_state.phase == GamePhase::PreFlop {
                                let p_idx = room.game_state.player_indices.get(&player_id);
                                if let Some(idx) = p_idx {
                                    let hands = room.game_state.player_cards[*idx];
                                    only_messages.push(ServerMessage::PlayerHand {
                                        hands: (hands.0.unwrap(), hands.1.unwrap()),
                                    });
                                }
                            }
                            vec![]
                        }
                        _ => vec![ServerMessage::Error { message: "该功能暂未实现".to_string() }]
                    }
                };

                // 广播消息
                for msg in broadcast_messages {
                    match &msg {
                        ServerMessage::Error { .. } => {
                            // 错误消息只发给当前玩家
                            let _ = tx.send(msg).await;
                        }
                        _ => {
                            broadcast(&targets, &msg, None).await;
                        }
                    }
                }
                // 发送仅发给当前玩家的消息
                for msg in only_messages {
                    let _ = tx.send(msg).await;
                }
            } else {
                let _ = tx.send(ServerMessage::Error { message: "请先加入或创建房间".to_string() }).await;
            }
        }
    }
}


/// 玩家断开连接后的处理
async fn handle_disconnect(state: SharedState, room_id: RoomId, player_id: PlayerId) {
    let delete_room;

    let targets;
    let mut update_state_msg = None;
    let mut host_transfer_msg = None;
    let mut host_transfer_info = None;
    {
        let mut room = state.rooms.get_mut(&room_id).unwrap();

        // 从连接映射中移除
        room.players.remove(&player_id);
        targets = create_msg_targets(&room.players);

        // 更新游戏状态中的玩家为 Offline
        if let Some(p) = room.game_state.players.get_mut(&player_id) {
            p.state = PlayerState::Offline;
            update_state_msg = Some(ServerMessage::PlayerUpdated { player: p.clone() });
        }

        // 如果房主断开，转移房主权限
        if player_id == room.host_id {
            if let Some(new_host_id) = room.players.keys().next().cloned() {
                room.host_id = new_host_id;
                host_transfer_msg = Some(ServerMessage::Info {
                    message: format!(
                        "房主已断开，新房主是 {}",
                        room.game_state.players.get(&new_host_id)
                            .map_or("未知玩家", |p| &p.nickname)
                    ),
                });
                host_transfer_info = Some(format!("房间 {} 的房主已转移给 {}", room_id, new_host_id));
            }
        }

        // 判断是否清空房间
        delete_room = room.players.is_empty();
    }

    info!("玩家 {} 从房间 {} 断开连接", player_id, room_id);

    if delete_room {
        state.rooms.remove(&room_id);
        info!("房间 {} 已空，已被移除", room_id);
    }

    if let Some(msg) = update_state_msg {
        broadcast(&targets, &msg, None).await;
    }
    if let Some(msg) = host_transfer_msg {
        broadcast(&targets, &msg, None).await;
        info!("{}", host_transfer_info.unwrap());
    }
}


/// 向房间内所有玩家广播消息
async fn broadcast(
    targets: &Vec<(PlayerId, mpsc::Sender<ServerMessage>)>,
    message: &ServerMessage,
    exclude: Option<PlayerId>,
) {
    for (player_id, sender) in targets {
        if Some(*player_id) == exclude {
            continue;
        }
        if sender.send(message.clone()).await.is_err() {
            // 发送失败，说明该玩家也断开了，后续由其自己的 handle_socket 任务处理
            tracing::warn!("向玩家 {} 发送消息失败（可能已断开）", player_id);
        }
    }
}

fn create_msg_targets(players: &HashMap<PlayerId, PlayerConnection>) -> Vec<(PlayerId, mpsc::Sender<ServerMessage>)> {
    players.iter().map(|(player_id, conn)|
        (*player_id, conn.sender.clone())
    ).collect()
}
