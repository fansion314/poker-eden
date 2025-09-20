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
    /// 客户端自己的玩家ID。
    my_id: Option<PlayerId>,
    /// 用户在输入框中输入的当前文本。
    input: String,
    /// 当轮到自己行动时，服务器会发送过来当前合法的动作列表。
    valid_actions: Vec<PlayerActionType>,
    /// 从服务器收到的最后一条错误信息或提示信息。
    last_error: Option<String>,
    /// 是否显示日志视图的标志。
    show_log: bool,
    /// 存储所有发送和接收的原始消息，用于调试。
    log_messages: Vec<String>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            ui_state: ClientUiState::Login, // 默认启动时是登录界面
            game_state: None,
            my_id: None,
            input: String::new(),
            valid_actions: vec![],
            last_error: None,
            show_log: false,
            log_messages: Vec::new(),
        }
    }
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

    // --- App 状态 & MPSC Channel ---
    let app = Arc::new(Mutex::new(App::default()));
    let (tx, rx) = mpsc::channel::<ClientMessage>(32);

    // --- 网络任务 ---
    let app_for_network = app.clone();
    tokio::spawn(network_task(app_for_network, rx));

    // --- 主UI循环 ---
    loop {
        terminal.draw(|f| ui(f, &mut app.lock().unwrap()))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                let mut app_guard = app.lock().unwrap();
                match key.code {
                    KeyCode::Enter => {
                        let input = app_guard.input.drain(..).collect::<String>();
                        let msg_to_send = match app_guard.ui_state {
                            // 如果在登录界面，解析登录命令
                            ClientUiState::Login => parse_login_input(&input),
                            // 如果在房间内，解析房间内命令（坐下、游戏动作等）
                            ClientUiState::InRoom => {
                                parse_in_room_input(&input, &app_guard)
                            }
                        };

                        if let Some(msg) = msg_to_send {
                            app_guard.log_messages.push(format!("[SEND] {:?}", msg));
                            let tx_clone = tx.clone();
                            tokio::spawn(async move {
                                tx_clone.send(msg).await.ok();
                            });
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
async fn network_task(app: Arc<Mutex<App>>, mut rx: mpsc::Receiver<ClientMessage>) {
    let server_addr = "127.0.0.1:25917";
    let url = url::Url::parse(&format!("ws://{}/ws", server_addr)).unwrap();

    let ws_stream = match tokio_tungstenite::connect_async(url.as_str()).await {
        Ok((stream, _)) => stream,
        Err(e) => {
            let mut app_guard = app.lock().unwrap();
            app_guard.last_error = Some(format!("连接服务器失败: {}", e));
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
                    app_guard.last_error = Some("与服务器的连接已断开。".to_string());
                    break;
                }
            }
            Some(Ok(msg)) = ws_receiver.next() => {
                if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                    let mut app_guard = app.lock().unwrap();
                    app_guard.log_messages.push(format!("[RECV] {}", text));
                    if let Ok(server_msg) = serde_json::from_str::<ServerMessage>(&text) {
                        handle_server_message(&mut app_guard, server_msg);
                    }
                } else if msg.is_close() {
                    let mut app_guard = app.lock().unwrap();
                    app_guard.last_error = Some("服务器已关闭连接。".to_string());
                    break;
                }
            }
            else => break,
        }
    }
}

/// 处理从服务器收到的消息，并据此更新应用程序的状态。
fn handle_server_message(app: &mut App, msg: ServerMessage) {
    match msg {
        // 成功加入房间后，将UI状态切换到 InRoom
        ServerMessage::RoomJoined { your_id, game_state, .. } => {
            app.my_id = Some(your_id);
            app.game_state = Some(game_state);
            app.ui_state = ClientUiState::InRoom; // 切换UI状态
        }
        ServerMessage::GameStateSnapshot(new_state) => app.game_state = Some(new_state),
        ServerMessage::PlayerJoined { player } => {
            if let Some(gs) = &mut app.game_state { gs.players.insert(player.id, player); }
        }
        ServerMessage::PlayerLeft { player_id } => {
            if let Some(gs) = &mut app.game_state {
                gs.players.remove(&player_id);
                gs.seated_players.retain(|id| id != &player_id);
            }
        }
        ServerMessage::PlayerUpdated { player } => {
            if let Some(gs) = &mut app.game_state {
                if player.state == PlayerState::Waiting {
                    app.log_messages.push(format!("玩家 {} 已加入房间", player.nickname));
                    gs.seated_players.insert(gs.find_insertion_index(player.seat_id.unwrap()), player.id);
                } else if player.state == PlayerState::Offline {
                    app.log_messages.push(format!("玩家 {} 已退出房间", player.nickname));
                    if let Some(idx) = gs.seated_players.iter().position(|id| id == &player.id) {
                        gs.seated_players.remove(idx);
                    }
                }
                if let Some(p) = gs.players.get_mut(&player.id) {
                    *p = player;
                }
            }
        }
        ServerMessage::HandStarted { hand_player_order, .. } => {
            if let Some(gs) = &mut app.game_state {
                gs.hand_player_order = hand_player_order;
                gs.player_indices = gs.hand_player_order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
                gs.phase = GamePhase::PreFlop;
                gs.bets = vec![0; gs.hand_player_order.len()];
                for p in gs.players.values_mut() {
                    if gs.hand_player_order.contains(&p.id) { p.state = PlayerState::Playing; }
                }
            }
        }
        ServerMessage::PlayerActed { player_id, action, total_bet_this_round, new_stack, new_pot } => {
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
        ServerMessage::CommunityCardsDealt { phase, cards } => {
            if let Some(gs) = &mut app.game_state {
                gs.phase = phase;
                let start_idx = match phase {
                    GamePhase::Flop => 0,
                    GamePhase::Turn => 3,
                    GamePhase::River => 4,
                    _ => return,
                };
                if gs.community_cards.is_empty() { gs.community_cards = vec![None; 5]; }
                for (i, card) in cards.into_iter().enumerate() { gs.community_cards[start_idx + i] = Some(card); }
            }
        }
        ServerMessage::Showdown { results } => {
            if let Some(gs) = &mut app.game_state {
                gs.phase = GamePhase::Showdown;
                for result in results {
                    if let Some(p) = gs.players.get_mut(&result.player_id) { p.stack += result.winnings; }
                    if let (Some(p_idx), Some(cards)) = (gs.player_indices.get(&result.player_id), result.cards) {
                        gs.player_cards[*p_idx] = (Some(cards.0), Some(cards.1));
                    }
                }
            }
        }
        ServerMessage::Error { message } => app.last_error = Some(message),
        _ => {}
    }
}

/// 解析登录界面的输入
fn parse_login_input(input: &str) -> Option<ClientMessage> {
    let parts: Vec<&str> = input.trim().split_whitespace().collect();
    if parts.is_empty() { return None; }

    match parts[0].to_lowercase().as_str() {
        "create" if parts.len() == 2 => {
            Some(ClientMessage::CreateRoom { nickname: parts[1].to_string() })
        }
        "join" if parts.len() == 3 => {
            if let Ok(room_id) = Uuid::from_str(parts[1]) {
                Some(ClientMessage::JoinRoom { room_id, nickname: parts[2].to_string() })
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

    if !is_seated {
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
                if app.valid_actions.iter().any(|a| matches!(a, PlayerActionType::Check | PlayerActionType::Call(_))) {
                    Some(PlayerAction::Call.into())
                } else { None }
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
            Constraint::Length(6), // 指令
            Constraint::Length(3), // 输入框
            Constraint::Percentage(40),
        ].as_ref())
        .split(f.size());

    let instructions_text = vec![
        Spans::from(Span::styled("欢迎来到德州扑克客户端", Style::default().add_modifier(Modifier::BOLD))),
        Spans::from(""),
        Spans::from("  create <你的昵称>"),
        Spans::from("  join <房间ID> <你的昵称>"),
    ];
    let instructions = Paragraph::new(instructions_text)
        .block(Block::default().borders(Borders::ALL).title("指令").border_type(BorderType::Rounded))
        .alignment(Alignment::Left);
    f.render_widget(instructions, chunks[1]);

    let input = Paragraph::new(app.input.as_ref())
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::ALL).title("输入").border_type(BorderType::Rounded));
    f.render_widget(input, chunks[2]);
    f.set_cursor(chunks[2].x + app.input.len() as u16 + 1, chunks[2].y + 1);
}

/// 绘制游戏内界面
fn draw_ingame_screen<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), Constraint::Length(5), Constraint::Min(10),
            Constraint::Length(3), Constraint::Length(3),
        ].as_ref())
        .split(f.size());

    if let Some(gs) = &mut app.game_state {
        draw_top_info(f, gs, chunks[0]);
        draw_community_cards(f, gs, chunks[1]);
        draw_players_table(f, gs, app.my_id, chunks[2]);
        draw_actions_and_input(f, app, chunks[3], chunks[4]);
    } else {
        let block = Block::default().title("正在加载房间信息...").borders(Borders::ALL);
        f.render_widget(block, f.size());
    }
}

fn draw_top_info<B: Backend>(f: &mut Frame<B>, gs: &GameState, area: Rect) {
    let pot_text = format!("奖池: ${}", gs.pot);
    let phase_text = format!("阶段: {:?}", gs.phase);
    let pot_paragraph = Paragraph::new(pot_text)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().title(phase_text).borders(Borders::ALL).border_type(BorderType::Rounded))
        .alignment(Alignment::Center);
    f.render_widget(pot_paragraph, area);
}

fn draw_community_cards<B: Backend>(f: &mut Frame<B>, gs: &GameState, area: Rect) {
    let cards_str: Vec<String> = gs.community_cards.iter()
        .map(|c| c.map_or(" ? ".to_string(), |card| card.to_string())).collect();
    let text = Spans::from(
        cards_str.join(" ").split_whitespace().map(|s| {
            Span::styled(format!(" {} ", s), Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD))
        }).collect::<Vec<Span>>(),
    );
    let paragraph = Paragraph::new(text)
        .block(Block::default().title("公共牌").borders(Borders::ALL).border_type(BorderType::Rounded))
        .alignment(Alignment::Center).wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

fn draw_players_table<B: Backend>(f: &mut Frame<B>, gs: &GameState, my_id: Option<PlayerId>, area: Rect) {
    let header_cells = ["座位", "玩家", "筹码", "下注", "手牌", "状态"]
        .iter().map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));
    let header = Row::new(header_cells).style(Style::default().bg(Color::DarkGray));
    let dealer_id = gs.hand_player_order.get(0);
    let rows = gs.seated_players.iter().map(|player_id| {
        let Some(player) = gs.players.get(player_id) else {
            return Row::new(vec![Cell::from("Error: Player not found")]);
        };
        let is_me = my_id == Some(*player_id);
        let is_dealer = dealer_id == Some(player_id);
        let is_thinking = gs.current_player_id() == Some(*player_id);
        let p_idx_opt = gs.player_indices.get(player_id);
        let bet = p_idx_opt.map_or(0, |idx| gs.bets.get(*idx).cloned().unwrap_or(0));
        let cards_tuple = p_idx_opt.map_or((None, None), |idx| gs.player_cards.get(*idx).cloned().unwrap_or((None, None)));
        let cards_str = match cards_tuple {
            (Some(c1), Some(c2)) => format!("[{} {}]", c1, c2),
            _ => "[ ? ? ]".to_string(),
        };
        let status_str = if is_thinking { "思考中...".to_string() } else { format!("{:?}", player.state) };
        let mut name = player.nickname.clone();
        if is_dealer { name.push_str(" (庄家)"); }
        if is_me { name.push_str(" (你)"); }
        let row_style = if is_thinking { Style::default().bg(Color::LightCyan).fg(Color::Black) } else if is_me { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
        Row::new(vec![
            Cell::from(player.seat_id.map_or("-".to_string(), |s| s.to_string())),
            Cell::from(name), Cell::from(format!("${}", player.stack)), Cell::from(format!("${}", bet)),
            Cell::from(cards_str), Cell::from(status_str),
        ]).style(row_style)
    });
    let table = Table::new(rows).header(header)
        .block(Block::default().borders(Borders::ALL).title("玩家列表").border_type(BorderType::Rounded))
        .widths(&[
            Constraint::Percentage(5), Constraint::Percentage(35), Constraint::Percentage(15),
            Constraint::Percentage(10), Constraint::Percentage(15), Constraint::Percentage(20),
        ]);
    f.render_widget(table, area);
}

fn draw_actions_and_input<B: Backend>(f: &mut Frame<B>, app: &App, actions_area: Rect, input_area: Rect) {
    let is_seated = app.my_id.map_or(false, |my_id| {
        app.game_state.as_ref().map_or(false, |gs| gs.seated_players.contains(&my_id))
    });

    let actions_text = if !app.valid_actions.is_empty() {
        let parts: Vec<String> = app.valid_actions.iter().map(|a| match a {
            PlayerActionType::Fold => "[F]弃牌".to_string(),
            PlayerActionType::Check => "[C]过牌".to_string(),
            PlayerActionType::Call(amount) => format!("[C]跟注 ${}", amount),
            PlayerActionType::BetOrRaise(min_amount) => format!("[R]加注 ${}+", min_amount),
        }).collect();
        format!("轮到你! 可用动作: {}", parts.join(", "))
    } else if !is_seated {
        "您正在观战。输入 `seat <座位号> <筹码>` 来坐下。".to_string()
    } else if let Some(err) = &app.last_error {
        err.clone()
    } else {
        "等待其他玩家行动...".to_string()
    };

    let actions_paragraph = Paragraph::new(actions_text)
        .style(Style::default().fg(Color::Green))
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
