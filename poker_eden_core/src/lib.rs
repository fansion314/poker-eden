//! # 德州扑克核心逻辑库
//!
//! 这个 `core` crate 包含了德州扑克游戏的所有核心状态管理、
//! 游戏逻辑、牌力评估以及客户端-服务器通信消息的定义。
//! 它的设计目标是与具体实现（如网络服务器、客户端UI）解耦，
//! 使其可以被任何上层应用复用。

mod card;
mod logic;
mod message;
mod state;

pub use card::*;

pub use message::*;

pub use state::*;

