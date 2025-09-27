use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::{SinkExt, StreamExt};
use poker_eden_core::*;
use std::{
    error::Error,
    io,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Spans, Text},
    widgets::{
        Block, BorderType, Borders, Cell, List, ListItem, Paragraph, Row, Table, Wrap,
    },
    Frame, Terminal,
};
use uuid::Uuid;

// --- 应用程序状态 ---

/// 用于管理UI显示哪个界面的状态机
#[derive(PartialEq, Debug)]
enum ClientUiState {
    Login,  // 登录/选择房间界面
    InRoom, // 在房间内（包括观战和游戏）
}

/// 这个结构体持有客户端运行所需的所有状态。
struct App {
    /// 控制当前显示哪个UI界面。
    ui_state: ClientUiState,
    /// 当前的游戏状态，从服务器接收。如果没有连接或游戏未开始，则为 None。
    game_state: Option<GameState>,
    /// 连接到的服务器地址
    server_addr: Option<String>,
    /// 用于向网络任务发送消息的发送器。
    msg_sender: Option<mpsc::Sender<ClientMessage>>,
    /// 创建房间后生成的分享信息。
    share_info: Option<String>,
    /// 客户端自己的玩家ID。
    my_id: Option<PlayerId>,
    /// 房主ID
    host_id: Option<PlayerId>,

    // 游戏过程中的状态
    /// 客户端当前的牌型
    hand_ranks: Vec<Option<HandRank>>,
    /// 上一局的筹码
    last_stack: Vec<u32>,
    /// 当轮到自己行动时，服务器会发送过来当前合法的动作列表。
    valid_actions: Vec<PlayerActionType>,

    /// 用户在输入框中输入的当前文本。
    input: String,
    /// 从服务器收到的最后一条错误信息或提示信息。
    last_msg: Option<String>,
    /// 是否显示日志视图的标志。
    show_log: bool,
    /// 存储所有发送和接收的原始消息，用于调试。
    log_messages: Vec<String>,
    should_refresh: bool,  // 是否需要刷新UI
}

impl Default for App {
    fn default() -> Self {
        Self {
            ui_state: ClientUiState::Login, // 默认启动时是登录界面
            game_state: None,
            server_addr: None,
            msg_sender: None,
            share_info: None,
            my_id: None,
            host_id: None,
            hand_ranks: vec![],
            last_stack: vec![],
            input: String::new(),
            valid_actions: vec![],
            last_msg: None,
            show_log: false,
            log_messages: Vec::new(),
            should_refresh: true,
        }
    }
}

/// 用于解析登录界面输入的命令
enum LoginCommand {
    Create { server_addr: String, nickname: String },
    Join { server_addr: String, room_id: RoomId, nickname: String },
}

// 应用程序的入口点
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // --- 设置终端 ---
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // --- App 状态 ---
    let app = Arc::new(Mutex::new(App::default()));

    // --- 主UI循环 ---
    loop {
        terminal.draw(|f| ui(f, &mut app.lock().unwrap()))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                let mut app_guard = app.lock().unwrap();
                match key.code {
                    KeyCode::Enter => {
                        let input = app_guard.input.drain(..).collect::<String>();
                        match app_guard.ui_state {
                            ClientUiState::Login => {
                                if let Some(login_cmd) = parse_login_input(&input) {
                                    let (tx, rx) = mpsc::channel(32);
                                    app_guard.msg_sender = Some(tx.clone());

                                    let (server_addr, initial_msg) = match login_cmd {
                                        LoginCommand::Create { server_addr, nickname } => {
                                            (server_addr, ClientMessage::CreateRoom { nickname })
                                        }
                                        LoginCommand::Join { server_addr, room_id, nickname } => {
                                            (server_addr, ClientMessage::JoinRoom { room_id, nickname })
                                        }
                                    };

                                    app_guard.server_addr = Some(server_addr.clone());
                                    let app_for_network = app.clone();
                                    tokio::spawn(network_task(app_for_network, tx.clone(), rx, server_addr));

                                    // 发送第一条消息 (创建或加入)
                                    tokio::spawn(async move {
                                        tx.send(initial_msg).await.ok();
                                    });
                                }
                            }
                            ClientUiState::InRoom => {
                                if let (Some(msg), Some(tx)) = (parse_in_room_input(&input, &app_guard), app_guard.msg_sender.as_ref()) {
                                    let _ = tx.try_send(msg);
                                }
                            }
                        }
                    }
                    KeyCode::Char(c) => app_guard.input.push(c),
                    KeyCode::Backspace => { app_guard.input.pop(); }
                    KeyCode::Tab => app_guard.show_log = !app_guard.show_log,
                    KeyCode::Esc => break,
                    _ => {}
                }
            }
        }
    }

    // --- 恢复终端 ---
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}

/// 独立的网络任务，处理所有与服务器的通信。
async fn network_task(app: Arc<Mutex<App>>, tx: mpsc::Sender<ClientMessage>, mut rx: mpsc::Receiver<ClientMessage>, server_addr: String) {
    let url = url::Url::parse(&format!("ws://{}/ws", server_addr)).unwrap();

    let ws_stream = match tokio_tungstenite::connect_async(url.as_str()).await {
        Ok((stream, _)) => stream,
        Err(e) => {
            let mut app_guard = app.lock().unwrap();
            app_guard.last_msg = Some(format!("连接服务器失败: {}", e));
            return;
        }
    };
    app.lock().unwrap().log_messages.push("已连接到服务器".to_string());

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    loop {
        tokio::select! {
            Some(msg_to_send) = rx.recv() => {
                let msg_text = serde_json::to_string(&msg_to_send).unwrap();
                app.lock().unwrap().log_messages.push(format!("[SEND_TO_SERVER] {}", msg_text));
                if ws_sender.send(tokio_tungstenite::tungstenite::Message::Text(msg_text.into())).await.is_err() {
                    let mut app_guard = app.lock().unwrap();
                    app_guard.last_msg = Some("与服务器的连接已断开。".to_string());
                    break;
                }
            }
            Some(Ok(msg)) = ws_receiver.next() => {
                if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                    let mut app_guard = app.lock().unwrap();
                    app_guard.log_messages.push(format!("[RECV] {}", text));
                    if let Ok(server_msg) = serde_json::from_str::<ServerMessage>(&text) {
                        let ret_msgs = handle_server_message(&mut app_guard, server_msg);
                        for msg in ret_msgs {
                            let _ = tx.try_send(msg);
                        }
                    }
                } else if msg.is_close() {
                    let mut app_guard = app.lock().unwrap();
                    app_guard.last_msg = Some("服务器已关闭连接。".to_string());
                    break;
                }
            }
            else => break,
        }
    }
}

/// 处理从服务器收到的消息，并据此更新应用程序的状态。
fn handle_server_message(app: &mut App, msg: ServerMessage) -> Vec<ClientMessage> {
    let mut ret_msgs = vec![];
    app.last_msg = None; // 收到任何消息都清除上一条错误
    app.should_refresh = true;
    match msg {
        // 成功加入房间后，将UI状态切换到 InRoom
        ServerMessage::RoomJoined { your_id, game_state, host_id, .. } => {
            app.my_id = Some(your_id);
            app.game_state = Some(game_state.clone());
            app.host_id = Some(host_id);
            app.ui_state = ClientUiState::InRoom; // 切换UI状态

            let playing_num = game_state.hand_player_order.len();
            app.hand_ranks = vec![None; playing_num];
            app.last_stack = vec![0; playing_num];

            // 如果是房主，生成分享链接
            if app.my_id == app.host_id {
                let share_addr = app.server_addr.as_ref().cloned().unwrap_or_default();
                app.share_info = Some(format!("分享信息: join {} {}", share_addr, game_state.room_id));
            }
        }
        ServerMessage::GameStateSnapshot(new_state) => app.game_state = Some(new_state),
        ServerMessage::PlayerJoined { player } => {
            if let Some(gs) = &mut app.game_state { gs.players.insert(player.id, player); }
        }
        ServerMessage::PlayerLeft { player_id } => {
            if let Some(gs) = &mut app.game_state {
                gs.players.get_mut(&player_id).unwrap().state = PlayerState::Offline;
            }
        }
        ServerMessage::PlayerUpdated { player } => {
            if let Some(gs) = &mut app.game_state {
                // 根据玩家状态变化，更新 seated_players 列表
                if player.state == PlayerState::Waiting {
                    // 如果玩家不在就座列表，则加入
                    if let Some(idx) = gs.seated_players.iter().position(|p| *p == player.id) {
                        gs.seated_players.remove(idx);
                        if let Some(i) = gs.player_indices.get(&player.id) {
                            app.last_stack[*i] = player.stack;
                        }
                    }
                    app.log_messages.push(format!("玩家 {} 已坐下准备游戏", player.nickname));
                    gs.seated_players.insert(gs.find_insertion_index(player.seat_id.unwrap()), player.id);
                } else if player.state == PlayerState::SittingOut {
                    // 如果玩家在就座列表，则移除
                    app.log_messages.push(format!("玩家 {} 离席", player.nickname));
                    if let Some(idx) = gs.seated_players.iter().position(|id| id == &player.id) {
                        gs.seated_players.remove(idx);
                    }
                }

                // 总是更新玩家在主列表中的数据
                if let Some(p) = gs.players.get_mut(&player.id) {
                    *p = player;
                }
            }
        }
        ServerMessage::HandStarted { seated_players, hand_player_order } => {
            if let Some(gs) = &mut app.game_state {
                app.share_info = None; // 游戏开始后清除分享信息
                gs.seated_players = seated_players;
                gs.hand_player_order = hand_player_order;
                gs.player_indices = gs.hand_player_order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
                gs.phase = GamePhase::PreFlop;
                gs.pot = 0;
                gs.bets = vec![0; gs.hand_player_order.len()];
                gs.last_bet = 0;
                gs.community_cards = vec![None; 5];
                gs.player_cards = vec![(None, None); gs.hand_player_order.len()];
                app.hand_ranks = vec![None; gs.hand_player_order.len()];
                for p in gs.players.values_mut() {
                    if gs.hand_player_order.contains(&p.id) { p.state = PlayerState::Playing; }
                }
                for player_id in gs.seated_players.iter() {
                    if let Some(p) = gs.players.get_mut(player_id) {
                        if p.state == PlayerState::Offline || p.stack == 0 {
                            p.state = PlayerState::SittingOut;
                        }
                    }
                }
                app.last_stack = gs.hand_player_order.iter().map(|p| {
                    gs.players.get(&p).unwrap().stack
                }).collect();
                ret_msgs.push(ClientMessage::GetMyHand);
            }
        }
        ServerMessage::PlayerHand { hands } => {
            if let Some(gs) = &mut app.game_state {
                if let Some(idx) = gs.player_indices.get(&app.my_id.unwrap()) {
                    gs.player_cards[*idx] = (Some(hands.0), Some(hands.1))
                }
            }
        }
        ServerMessage::PlayerActed { player_id, action, total_bet: total_bet_this_round, new_stack, new_pot } => {
            if let Some(gs) = &mut app.game_state {
                gs.pot = new_pot;
                if let Some(p_idx) = gs.player_indices.get(&player_id) {
                    gs.bets[*p_idx] = total_bet_this_round;
                    if let Some(p) = gs.players.get_mut(&player_id) {
                        p.stack = new_stack;
                        match action {
                            PlayerAction::Fold => p.state = PlayerState::Folded,
                            _ => { if p.stack == 0 && p.state != PlayerState::Folded { p.state = PlayerState::AllIn } }
                        }
                    }
                }
                gs.max_bet = gs.max_bet.max(total_bet_this_round);
            }
        }
        ServerMessage::NextToAct { player_id, valid_actions } => {
            if let Some(gs) = &mut app.game_state {
                if let Some(idx) = gs.player_indices.get(&player_id) { gs.cur_player_idx = *idx; }
            }
            if app.my_id == Some(player_id) { app.valid_actions = valid_actions; } else { app.valid_actions.clear(); }
        }
        ServerMessage::CommunityCardsDealt { phase, cards, last_bet } => {
            if let Some(gs) = &mut app.game_state {
                gs.phase = phase;
                let start_idx = match phase {
                    GamePhase::Flop => 0,
                    GamePhase::Turn => 3,
                    GamePhase::River => 4,
                    _ => return vec![],
                };
                gs.last_bet = last_bet;
                if gs.community_cards.is_empty() { gs.community_cards = vec![None; 5]; }
                for (i, card) in cards.into_iter().enumerate() { gs.community_cards[start_idx + i] = Some(card); }

                // 更新玩家的牌型
                let community_cards = gs.community_cards.iter().map_while(|card| {
                    card.clone()
                }).collect::<Vec<_>>();
                for (p_idx, player_card) in gs.player_cards.iter().enumerate() {
                    if let (Some(card1), Some(card2)) = player_card {
                        let mut cards = community_cards.clone();
                        cards.push(*card1);
                        cards.push(*card2);
                        let rank = find_best_hand(&cards);
                        app.hand_ranks[p_idx] = Some(rank);
                    }
                }
            }
        }
        ServerMessage::Showdown { results } => {
            if let Some(gs) = &mut app.game_state {
                gs.phase = GamePhase::Showdown;
                for result in results {
                    if let Some(p) = gs.players.get_mut(&result.player_id) {
                        if result.winnings > 0 {
                            p.stack += result.winnings;
                            p.wins += 1;
                        }
                    }
                    if let (Some(p_idx), Some(cards), Some(hand_rank))
                        = (gs.player_indices.get(&result.player_id), result.cards, result.hand_rank) {
                        gs.player_cards[*p_idx] = (Some(cards.0), Some(cards.1));
                        app.hand_ranks[*p_idx] = Some(hand_rank);
                    }
                }
                for p in gs.hand_player_order.iter() {
                    if let Some(p) = gs.players.get_mut(p) {
                        if p.stack == 0 {
                            p.losses += 1;
                            p.state = PlayerState::Offline;
                        };
                    }
                }
            }
        }
        ServerMessage::BetReturned { player_id, amount, new_stack } => {
            if let Some(gs) = &mut app.game_state {
                if let Some(p) = gs.players.get_mut(&player_id) {
                    p.stack = new_stack;
                }
                gs.pot -= amount;
            }
        }
        ServerMessage::Error { message } | ServerMessage::Info { message } => app.last_msg = Some(message),
    }
    ret_msgs
}

/// 解析登录界面的输入
fn parse_login_input(input: &str) -> Option<LoginCommand> {
    let parts: Vec<&str> = input.trim().split_whitespace().collect();
    if parts.len() < 3 { return None; }

    match parts[0].to_lowercase().as_str() {
        "create" if parts.len() == 3 => {
            // 简单验证地址格式，但不做完整解析
            if parts[1].contains(':') {
                Some(LoginCommand::Create { server_addr: parts[1].to_string(), nickname: parts[2].to_string() })
            } else { None }
        }
        "join" if parts.len() == 4 => {
            if let Ok(room_id) = Uuid::from_str(parts[2]) {
                if parts[1].contains(':') {
                    Some(LoginCommand::Join { server_addr: parts[1].to_string(), room_id, nickname: parts[3].to_string() })
                } else { None }
            } else { None }
        }
        _ => None,
    }
}

/// 解析在房间内的输入（坐下或游戏动作）
fn parse_in_room_input(input: &str, app: &App) -> Option<ClientMessage> {
    let parts: Vec<&str> = input.trim().split_whitespace().collect();
    if parts.is_empty() { return None; }

    // 检查玩家是否已经就座
    let is_seated = app.my_id.map_or(false, |my_id| {
        app.game_state.as_ref().map_or(false, |gs| gs.seated_players.contains(&my_id))
    });

    // 检查是否为房主、已就座、在等待阶段，以解析 "start" 命令
    if app.my_id == app.host_id && is_seated && parts[0].to_lowercase() == "start"
        && app.game_state.as_ref().map_or(false, |gs| {
        gs.phase == GamePhase::WaitingForPlayers || gs.phase == GamePhase::Showdown
    }) {
        return Some(ClientMessage::StartHand);
    }

    let is_lose_game = app.game_state.as_ref().map_or(false, |gs| {
        gs.players.get(&app.my_id.unwrap()).map_or(false, |p| p.state == PlayerState::Offline)
    });

    if !is_seated || is_lose_game {
        // 如果未就座，只解析 "seat" 命令
        if parts[0].to_lowercase() == "seat" && parts.len() == 3 {
            if let (Ok(seat_id), Ok(stack)) = (parts[1].parse::<u8>(), parts[2].parse::<u32>()) {
                return Some(ClientMessage::RequestSeat { seat_id, stack });
            }
        }
    } else {
        // 如果已就座，解析游戏动作
        return match parts[0].to_lowercase().as_str() {
            "f" | "fold" => Some(PlayerAction::Fold.into()),
            "c" | "check" | "call" => {
                let mut is_check = false;
                let mut is_call = false;
                for valid_action in app.valid_actions.iter() {
                    match valid_action {
                        PlayerActionType::Check => {
                            is_check = true;
                            break;
                        }
                        PlayerActionType::Call(_) => {
                            is_call = true;
                            break;
                        }
                        _ => continue,
                    }
                }
                if is_check { Some(PlayerAction::Check.into()) } else if is_call { Some(PlayerAction::Call.into()) } else { None }
            }
            "b" | "r" | "bet" | "raise" => {
                if parts.len() > 1 {
                    if let Ok(amount) = parts[1].parse::<u32>() {
                        Some(PlayerAction::BetOrRaise(amount).into())
                    } else { None }
                } else { None }
            }
            _ => None,
        };
    }
    None
}

// --- UI 渲染 ---

/// 主UI绘制函数，根据客户端状态选择渲染哪个界面。
fn ui<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    if app.show_log {
        draw_log(f, app);
        return;
    }

    match app.ui_state {
        ClientUiState::Login => draw_login_screen(f, app),
        ClientUiState::InRoom => draw_ingame_screen(f, app),
    }
}

/// 绘制登录界面
fn draw_login_screen<B: Backend>(f: &mut Frame<B>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(8), // 指令
            Constraint::Length(3), // 输入框
            Constraint::Percentage(40),
        ].as_ref())
        .split(f.size());

    let instructions_text = vec![
        Spans::from(Span::styled("欢迎来到德州扑克客户端", Style::default().add_modifier(Modifier::BOLD))),
        Spans::from(""),
        Spans::from("->创建房间: create <服务器地址:端口> <你的昵称>"),
        Spans::from("  例如: create 127.0.0.1:25917 Alice"),
        Spans::from(""),
        Spans::from("->加入房间: join <服务器地址:端口> <房间ID> <你的昵称>"),
    ];
    let instructions = Paragraph::new(instructions_text)
        .block(Block::default().borders(Borders::ALL).title("指令").border_type(BorderType::Rounded))
        .alignment(Alignment::Left);
    f.render_widget(instructions, chunks[1]);

    let input_text = if let Some(err) = &app.last_msg {
        err.as_str()
    } else {
        app.input.as_ref()
    };
    let input_style = if app.last_msg.is_some() {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Yellow)
    };

    let input = Paragraph::new(input_text)
        .style(input_style)
        .block(Block::default().borders(Borders::ALL).title("输入").border_type(BorderType::Rounded));
    f.render_widget(input, chunks[2]);

    if app.last_msg.is_none() {
        f.set_cursor(chunks[2].x + app.input.len() as u16 + 1, chunks[2].y + 1);
    }
}

/// 绘制游戏内界面
fn draw_ingame_screen<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), Constraint::Length(5), Constraint::Min(10),
            if app.share_info.is_some() || app.last_msg.is_some() { Constraint::Length(4) } else { Constraint::Length(3) },
            Constraint::Length(3),
        ].as_ref())
        .split(f.size());

    if let Some(_) = &app.game_state {
        draw_top_info(f, app, chunks[0]);
        draw_community_cards(f, app, chunks[1]);
        draw_players_table(f, app, chunks[2]);
        draw_actions_and_input(f, app, chunks[3], chunks[4]);
        if app.should_refresh { app.should_refresh = false; }
    } else {
        let block = Block::default().title("正在加载房间信息...").borders(Borders::ALL);
        f.render_widget(block, f.size());
    }
}

fn draw_top_info<B: Backend>(f: &mut Frame<B>, app: &App, area: Rect) {
    let gs = app.game_state.as_ref().unwrap();
    let pot_text = format!("奖池: ${}", gs.pot);
    let phase_text = format!("阶段: {}", gs.phase);
    let owner_nickname = &gs.players.get(&app.host_id.unwrap()).unwrap().nickname;
    let room_text = format!("房间ID: {}  房主：{}  NLH ~ {}/{}", gs.room_id,
                            owner_nickname, gs.small_blind, gs.big_blind);
    let top_block = Block::default()
        .title(Span::styled(phase_text, Style::default()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);

    f.render_widget(top_block, area);

    // 在 Block 内部手动布局
    let inner_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .margin(1)
        .constraints([
            Constraint::Percentage(85),
            Constraint::Percentage(15),
        ])
        .split(area);

    let room_paragraph = Paragraph::new(room_text).alignment(Alignment::Left);
    let pot_paragraph = Paragraph::new(pot_text)
        .style(Style::default().fg(Color::Yellow))
        .alignment(Alignment::Right);

    f.render_widget(room_paragraph, inner_chunks[0]);
    f.render_widget(pot_paragraph, inner_chunks[1]);
}

fn draw_community_cards<B: Backend>(f: &mut Frame<B>, app: &App, area: Rect) {
    let Some(gs) = &app.game_state else { return };
    let text = if gs.phase == GamePhase::WaitingForPlayers {
        Spans::from(vec![])
    } else {
        let cards_str: Vec<String> = gs.community_cards.iter()
            .map(|c| c.map_or("___".to_string(), |card| {
                if app.should_refresh { "___".to_string() } else { card.to_string() }
            })).collect();
        Spans::from(
            cards_str.into_iter().map(|s| {
                let color = if s.contains('♥') || s.contains('♦') { Color::Red } else { Color::Black };
                Span::styled(format!(" {} ", s), Style::default().fg(color).bg(Color::White).add_modifier(Modifier::BOLD))
            }).collect::<Vec<Span>>(),
        )
    };
    let paragraph = Paragraph::new(text)
        .block(Block::default().title("公共牌").borders(Borders::ALL).border_type(BorderType::Rounded))
        .alignment(Alignment::Center).wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

// 修改了函数签名
fn draw_players_table<B: Backend>(f: &mut Frame<B>, app: &App, area: Rect) {
    let Some(gs) = &app.game_state else { return };
    let my_id = app.my_id;

    let header_cells = ["座位", "玩家", "胜", "负", "筹码", "下注", "手牌", "牌型", "状态"]
        .iter().map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));
    let header = Row::new(header_cells).style(Style::default().bg(Color::DarkGray));
    let dealer_id = if gs.hand_player_order.is_empty() { None } else { Some(gs.hand_player_order[0]) }; // 庄家是就座列表的第一个
    let show_stack_change = gs.phase == GamePhase::Showdown && !app.last_stack.iter().all(|x| *x == 0);
    let rows = gs.seated_players.iter().map(|player_id| {
        let Some(player) = gs.players.get(player_id) else {
            return Row::new(vec![Cell::from("Error: Player not found")]);
        };
        let is_me = my_id == Some(*player_id);
        let is_dealer = dealer_id == Some(*player_id);
        let is_thinking = gs.phase != GamePhase::Showdown && gs.current_player_id() == Some(*player_id);
        let p_idx_opt = gs.player_indices.get(player_id);
        let bet = p_idx_opt.map_or(0, |idx| {
            gs.bets.get(*idx).cloned().unwrap_or(0).saturating_sub(gs.last_bet)
        });
        let mut player_stack_str = format!("${}", player.stack);
        if show_stack_change && let Some(idx) = p_idx_opt {
            let change_stack = player.stack as i32 - app.last_stack[*idx] as i32;
            if change_stack > 0 {
                player_stack_str.push_str(format!("(+${})", change_stack).as_str());
            } else if change_stack < 0 {
                player_stack_str.push_str(format!("(-${})", -change_stack).as_str());
            }
        }
        let cards_tuple = p_idx_opt.map_or((None, None), |idx| gs.player_cards.get(*idx).cloned().unwrap_or((None, None)));
        let cards_spans: Vec<Span> = match cards_tuple {
            (Some(c1), Some(c2)) if !app.should_refresh => {
                [c1, c2].into_iter().map(|c| {
                    let color = if c.suit == Suit::Heart || c.suit == Suit::Diamond { Color::Red } else { Color::Black };
                    Span::styled(format!(" {} ", c), Style::default().fg(color).bg(Color::White))
                }).collect()
            }
            _ => vec![Span::styled(" ___  ___ ", Style::default().fg(Color::Black).bg(Color::White))],
        };

        let cards_rank = p_idx_opt.map_or("".to_string(), |idx| {
            match app.hand_ranks.get(*idx).unwrap() {
                None => "".to_string(),
                Some(rank) => format!("{}", rank),
            }
        });
        let status_str = if is_thinking { "思考中...".to_string() } else { format!("{}", player.state) };
        let mut name = "".to_string();
        if is_me { name.push_str("[你]"); }
        name.push_str(player.nickname.as_str());
        if is_dealer { name.push_str(" (D)"); }
        let row_style = if is_thinking { Style::default().bg(Color::LightCyan).fg(Color::Black) } else if is_me { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
        Row::new(vec![
            Cell::from(player.seat_id.map_or("-".to_string(), |s| s.to_string())),
            Cell::from(name),
            Cell::from(if player.wins > 0 { format!("{}", player.wins) } else { "".to_string() }),
            Cell::from(if player.losses > 0 { format!("{}", player.losses) } else { "".to_string() }),
            Cell::from(player_stack_str),
            Cell::from(format!("${}", bet)),
            Cell::from(Spans::from(cards_spans)),
            Cell::from(cards_rank),
            Cell::from(status_str),
        ]).style(row_style)
    });
    let table = Table::new(rows).header(header)
        .block(Block::default().borders(Borders::ALL).title("玩家列表").border_type(BorderType::Rounded))
        .widths(&[
            Constraint::Percentage(5), Constraint::Percentage(17), Constraint::Percentage(4),
            Constraint::Percentage(4), Constraint::Percentage(16), Constraint::Percentage(10),
            Constraint::Percentage(14), Constraint::Percentage(11), Constraint::Percentage(15),
        ]);
    f.render_widget(table, area);
}

fn draw_actions_and_input<B: Backend>(f: &mut Frame<B>, app: &App, actions_area: Rect, input_area: Rect) {
    let is_seated = app.my_id.map_or(false, |my_id| {
        app.game_state.as_ref().map_or(false, |gs| gs.seated_players.contains(&my_id))
    });

    let is_lose_game = app.game_state.as_ref().map_or(false, |gs| {
        gs.players.get(&app.my_id.unwrap()).map_or(false, |p| p.state == PlayerState::Offline)
    });

    let game_phase = app.game_state.as_ref().map(|gs| gs.phase);
    let is_waiting_phase = game_phase == Some(GamePhase::WaitingForPlayers);
    let is_showdown_phase = game_phase == Some(GamePhase::Showdown);

    // 修改了UI提示逻辑
    let mut info_text = if !app.valid_actions.is_empty() && !is_showdown_phase {
        // Case 1: 轮到你行动
        let parts: Vec<String> = app.valid_actions.iter().map(|a| match a {
            PlayerActionType::Fold => "[f]弃牌(Fold)".to_string(),
            PlayerActionType::Check => "[c]过牌(Check)".to_string(),
            PlayerActionType::Call(amount) => format!("[c]跟注(Call) ${}", amount),
            PlayerActionType::Bet(min_amount) => format!("[b]下注(Bet) ${}+", min_amount),
            PlayerActionType::Raise(min_amount) => format!("[r]加注(Raise) ${}+", min_amount),
        }).collect();
        format!("轮到你! {}", parts.join(", "))
    } else if app.my_id == app.host_id && (is_waiting_phase || is_showdown_phase) {
        // Case 2: 你是房主，并且在等待阶段
        let share_info_str = app.share_info.as_deref().unwrap_or("");
        if is_seated {
            format!("{}\n你是房主。等待玩家加入... 输入 `start` 开始游戏。", share_info_str)
        } else {
            format!("{}\n你是房主。请先 `seat <座位号> <筹码>` 坐下才能开始游戏。", share_info_str)
        }
    } else if let Some(share_info) = &app.share_info {
        // Case 3: 你是普通玩家，在等待阶段
        share_info.clone()
    } else if !is_seated || is_lose_game {
        // Case 4: 你是旁观者
        "您正在观战。输入 `seat <座位号> <筹码>` 来坐下。".to_string()
    } else if is_showdown_phase {
        "本局游戏结束，等待房主开始下一局游戏🎮".to_string()
    } else {
        // Case 6: 默认等待信息
        "等待其他玩家行动...".to_string()
    };

    if let Some(err) = &app.last_msg {
        info_text = format!("消息：{}\n{}", err.as_str(), info_text);
    }

    let p_style = if app.last_msg.is_some() { Style::default().fg(Color::Red) } else { Style::default().fg(Color::White) };
    let actions_paragraph = Paragraph::new(info_text.trim_start_matches("\n"))
        .style(p_style)
        .block(Block::default().borders(Borders::ALL).title("可用动作 / 信息").border_type(BorderType::Rounded))
        .alignment(Alignment::Center);
    f.render_widget(actions_paragraph, actions_area);

    let input = Paragraph::new(app.input.as_ref())
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::ALL).title("输入").border_type(BorderType::Rounded));
    f.render_widget(input, input_area);
    f.set_cursor(input_area.x + app.input.len() as u16 + 1, input_area.y + 1);
}

fn draw_log<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    let log_items: Vec<ListItem> = app.log_messages.iter().rev()
        .map(|msg| ListItem::new(Text::from(msg.as_str()))).collect();
    let log_list = List::new(log_items)
        .block(Block::default().borders(Borders::ALL).title("日志 (按 Tab 关闭)").border_type(BorderType::Rounded))
        .style(Style::default().fg(Color::White));
    f.render_widget(log_list, f.size());
}

