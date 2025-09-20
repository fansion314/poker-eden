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
use parking_lot::{Mutex as P_Mutex, RwLock as P_RwLock};
use tokio::sync::{mpsc, RwLock};
use tracing::info;
use uuid::Uuid;

use poker_eden_core::{ClientMessage, GameState, Player, PlayerId, PlayerSecret, PlayerState, RoomId, ServerMessage};

// 服务器全局状态，使用 Arc<Mutex<...>> 实现线程安全共享
struct AppState {
    rooms: DashMap<RoomId, Arc<Room>>,
}

// 单个房间的状态
// 重要‼️：严格规定使用锁的顺序，避免死锁：
// players -> host_id -> game_state
struct Room {
    game_state: P_Mutex<GameState>,
    host_id: P_RwLock<PlayerId>,
    // 将 PlayerId 映射到具体的网络连接
    players: RwLock<HashMap<PlayerId, PlayerConnection>>,
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
    tracing_subscriber::fmt::init();

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
    info!("客户端连接关闭");
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
                game_state: P_Mutex::new(game_state),
                host_id: P_RwLock::new(player_id),
                players: RwLock::new(HashMap::new()),
            };
            room.players.get_mut().insert(player_id, PlayerConnection {
                secret: player_secret,
                sender: tx.clone(),
            });

            state.rooms.insert(room_id, Arc::new(room));

            info!("玩家 {} 创建了新房间 {}", player_id, room_id);
            *context = Some((room_id, player_id));
            let _ = tx.send(ServerMessage::RoomJoined {
                your_id: player_id,
                your_secret: player_secret,
                game_state: gs_for_client,
            }).await;
        }
        ClientMessage::JoinRoom { room_id, nickname } => {
            if context.is_some() {
                let _ = tx.send(ServerMessage::Error { message: "你已经在一个房间里了".to_string() }).await;
                return;
            }

            let room = match state.rooms.get(&room_id) {
                Some(r) => r.clone(),
                None => {
                    let _ = tx.send(ServerMessage::Error { message: "房间不存在".to_string() }).await;
                    return;
                }
            };

            let player_id = Uuid::new_v4();
            let player_secret = Uuid::new_v4();
            let gs_for_client;

            let player = Player {
                id: player_id,
                nickname,
                stack: 0,
                wins: 0,
                losses: 0,
                state: PlayerState::SittingOut,
                seat_id: None,
            };

            {  // r_players write lock
                let mut r_players = room.players.write().await;

                {  // r_game_state lock
                    let mut game_state = room.game_state.lock();
                    game_state.players.insert(player_id, player.clone());
                    gs_for_client = game_state.for_client(&player_id);
                }

                r_players.insert(player_id, PlayerConnection {
                    secret: player_secret,
                    sender: tx.clone(),
                });
            }

            info!("玩家 {} 加入了房间 {}", player_id, room_id);
            *context = Some((room_id, player_id));
            {  // r_players read lock
                // 广播给房间内其他玩家
                let join_msg = ServerMessage::PlayerJoined { player: player.clone() };
                broadcast(room.players.read().await.iter(), &join_msg, Some(player_id)).await;
            }
            let _ = tx.send(ServerMessage::RoomJoined {
                your_id: player_id,
                your_secret: player_secret,
                game_state: gs_for_client,
            }).await;
        }
        // ... 其他需要认证后才能执行的消息
        _ => {
            if let Some((room_id, player_id)) = context {
                let room = match state.rooms.get(room_id) {
                    None => {
                        let _ = tx.send(ServerMessage::Error { message: "房间不存在".to_string() }).await;
                        return;
                    }
                    Some(r) => r.clone(),
                };
                // 游戏逻辑处理
                let messages = match msg {
                    ClientMessage::StartHand => {
                        let host_id = *room.host_id.read();
                        if *player_id != host_id {
                            vec![ServerMessage::Error { message: "只有房主可以开始游戏".to_string() }]
                        } else {
                            room.game_state.lock().start_new_hand()
                        }
                    }
                    ClientMessage::PerformAction(action) => {
                        room.game_state.lock().handle_player_action(*player_id, action)
                    }
                    // TODO: 实现其他 ClientMessage 的处理
                    _ => vec![ServerMessage::Error { message: "该功能暂未实现".to_string() }]
                };

                // 广播消息
                for msg in messages {
                    match &msg {
                        ServerMessage::Error { .. } => {
                            // 错误消息只发给当前玩家
                            let _ = tx.send(msg).await;
                        }
                        ServerMessage::GameStateSnapshot(gs) => {
                            // 快照需要为每个玩家单独生成
                            for (pid, conn) in room.players.read().await.iter() {
                                let personalized_gs = gs.for_client(pid);
                                let _ = conn.sender.send(ServerMessage::GameStateSnapshot(personalized_gs)).await;
                            }
                        }
                        _ => {
                            broadcast(room.players.read().await.iter(), &msg, None).await;
                        }
                    }
                }
            } else {
                let _ = tx.send(ServerMessage::Error { message: "请先加入或创建房间".to_string() }).await;
            }
        }
    }
}


/// 玩家断开连接后的处理
async fn handle_disconnect(state: SharedState, room_id: RoomId, player_id: PlayerId) {
    info!("玩家 {} 从房间 {} 断开连接", player_id, room_id);
    let room = match state.rooms.get(&room_id) {
        None => return,
        Some(r) => r.clone(),
    };

    {  // r_players write lock
        let mut r_players = room.players.write().await;
        // 从连接映射中移除
        r_players.remove(&player_id);

        // 更新游戏状态中的玩家为 Offline
        let mut update_msg = None;
        {  // r_game_state lock
            let mut game_state = room.game_state.lock();
            if let Some(p) = game_state.players.get_mut(&player_id) {
                p.state = PlayerState::Offline;
                update_msg = Some(ServerMessage::PlayerUpdated { player: p.clone() });
            }
        }
        if let Some(msg) = update_msg {
            broadcast(r_players.iter(), &msg, None).await;
        }
    }

    {  // r_players read lock
        let r_players = room.players.read().await;

        // 如果房主断开，转移房主权限
        let host_id = *room.host_id.read();
        if player_id == host_id {
            if let Some(new_host_id) = r_players.keys().next().cloned() {
                *room.host_id.write() = new_host_id;
                let info_msg = ServerMessage::Info {
                    message: format!(
                        "房主已断开，新房主是 {}",
                        room.game_state.lock().players.get(&new_host_id)
                            .map_or("未知玩家", |p| &p.nickname)
                    ),
                };
                broadcast(r_players.iter(), &info_msg, None).await;
                info!("房间 {} 的房主已转移给 {}", room_id, new_host_id);
            }
        }

        // 判断是否清空房间
        if r_players.is_empty() {
            state.rooms.remove(&room_id);
            info!("房间 {} 已空，已被移除", room_id);
        }
    }
}


/// 向房间内所有玩家广播消息
async fn broadcast(
    players: impl Iterator<Item=(&PlayerId, &PlayerConnection)>,
    message: &ServerMessage,
    exclude: Option<PlayerId>,
) {
    for (player_id, conn) in players {
        if Some(*player_id) == exclude {
            continue;
        }
        if conn.sender.send(message.clone()).await.is_err() {
            // 发送失败，说明该玩家也断开了，后续由其自己的 handle_socket 任务处理
            tracing::warn!("向玩家 {} 发送消息失败（可能已断开）", player_id);
        }
    }
}
