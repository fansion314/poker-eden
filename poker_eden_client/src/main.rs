use futures_util::{SinkExt, StreamExt};
use std::io::Write;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use url::Url;

use poker_eden_core::{ClientMessage, PlayerAction, RoomId, ServerMessage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = Url::parse("ws://127.0.0.1:25917/ws").unwrap();

    println!("正在连接到: {}", url);
    let (ws_stream, _) = connect_async(url.as_str()).await.expect("无法连接");
    println!("连接成功!");

    let (mut write, mut read) = ws_stream.split();

    // 启动一个任务来处理从服务器接收的消息
    tokio::spawn(async move {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    match serde_json::from_str::<ServerMessage>(&text) {
                        Ok(server_msg) => {
                            // 简单地将收到的消息打印到控制台
                            println!("\n<-- [服务器消息]:\n{:#?}\n", server_msg);
                            print!("> "); // 重新显示输入提示符
                            std::io::stdout().flush().unwrap();
                        }
                        Err(e) => eprintln!("解析服务器消息失败: {}", e),
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("接收消息时出错: {}", e);
                    break;
                }
            }
        }
    });

    // 主任务处理用户输入
    let mut stdin = BufReader::new(tokio::io::stdin()).lines();

    println!("--- 德州扑克客户端 ---");
    println!("可用命令:");
    println!("  create <昵称>             - 创建一个新房间");
    println!("  join <房间ID> <昵称>      - 加入一个房间");
    println!("  start                     - 开始游戏 (仅房主)");
    println!("  fold                      - 弃牌");
    println!("  check                     - 过牌");
    println!("  call                      - 跟注");
    println!("  raise <金额>              - 加注到指定总额");
    println!("  exit                      - 退出");

    loop {
        print!("> ");
        std::io::stdout().flush().unwrap();

        let line = stdin.next_line().await?.unwrap_or_default();
        let parts: Vec<&str> = line.trim().split_whitespace().collect();
        let command = parts.get(0).cloned();

        let client_msg = match command {
            Some("create") => {
                let nickname = parts.get(1).unwrap_or(&"新玩家").to_string();
                Some(ClientMessage::CreateRoom { nickname })
            }
            Some("join") => {
                if parts.len() < 3 {
                    println!("用法: join <房间ID> <昵称>");
                    continue;
                }
                let room_id: RoomId = parts[1].parse().expect("无效的房间ID格式");
                let nickname = parts[2].to_string();
                Some(ClientMessage::JoinRoom { room_id, nickname })
            }
            Some("start") => Some(ClientMessage::StartHand),
            Some("fold") => Some(ClientMessage::PerformAction(PlayerAction::Fold)),
            Some("check") => Some(ClientMessage::PerformAction(PlayerAction::Check)),
            Some("call") => Some(ClientMessage::PerformAction(PlayerAction::Call)),
            Some("raise") => {
                if parts.len() < 2 {
                    println!("用法: raise <金额>");
                    continue;
                }
                let amount: u32 = parts[1].parse().expect("无效的金额");
                Some(ClientMessage::PerformAction(PlayerAction::BetOrRaise(amount)))
            }
            Some("exit") => {
                println!("正在断开连接...");
                break;
            }
            _ => {
                println!("未知命令: {}", line);
                continue;
            }
        };

        if let Some(msg) = client_msg {
            let payload = serde_json::to_string(&msg)?;
            write.send(Message::Text(payload.into())).await?;
        }
    }

    Ok(())
}
