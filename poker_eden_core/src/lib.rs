// This file is part of poker_eden.
//
// poker_eden is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// poker_eden is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with poker_eden. If not, see <https://www.gnu.org/licenses/>.
//
// Copyright (C) 2025 Peilin Fan <peilin.fan@foxmail.com>

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

