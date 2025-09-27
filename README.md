# Poker Eden - 联机德州扑克游戏

Poker Eden 是一个用 Rust 编写的联机德州扑克游戏项目。它实现了一个功能齐全的德州扑克核心逻辑库，以及一个基于 WebSocket
的客户端-服务器架构，并带有一个简单的终端用户界面。

## 项目特点

- **完整的德州扑克逻辑**: 实现了包括盲注、翻牌前、翻牌、转牌、河牌以及摊牌在内的完整游戏流程。
- **精确的牌力评估**: 能够从 5 到 7 张牌中准确找出最强的 5 张牌组合。
- **健壮的奖池分配**: 支持复杂的边池（Side Pot）计算，确保在有玩家全下（All-In）的情况下也能正确分配奖金。
- **清晰的模块化设计**: 项目被划分为三个独立的 crate：
    - `poker_eden_core`: 核心游戏逻辑，与具体实现解耦。
    - `poker_eden_server`: 基于 Axum 和 WebSocket 的游戏服务器。
    - `poker_eden_client`: 一个基于 `tui-rs` 的终端客户端。
- **异步架构**: 服务器和客户端均采用 `tokio` 实现异步通信，性能高效。

## 项目结构

```
.
├── poker_eden_client/ # 客户端 Crate
├── poker_eden_core/   # 核心逻辑 Crate
├── poker_eden_server/ # 服务器 Crate
└── Cargo.toml         # 工作区配置
```

- **`poker_eden_core`**: 包含了所有游戏的核心数据结构（如 `Card`, `HandRank`, `GameState`
  ）、游戏流程控制（下注、阶段推进）以及客户端-服务器通信消息的定义。
- **`poker_eden_server`**: 实现了一个 WebSocket 服务器，用于管理游戏房间、处理玩家连接和转发游戏逻辑。
- **`poker_eden_client`**: 提供一个简单的终端界面，允许玩家连接到服务器、加入游戏并进行交互。

## 如何运行

你需要安装 [Rust](https://www.rust-lang.org/tools/install) 环境。

### 1. 启动服务器

在项目根目录下执行以下命令：

```bash
cargo run -p poker_eden_server
```

服务器将默认在 `127.0.0.1:8080` 启动。

### 2. 启动客户端

打开一个新的终端窗口，在项目根目录下执行以下命令来启动一个客户端实例：

```bash
cargo run -p poker_eden_client
```

你可以启动多个客户端实例来模拟多人游戏。客户端启动后，将提示你输入服务器地址、房间号和昵称。

## 核心逻辑亮点

- **`HandRank` 枚举**: `poker_eden_core/src/card.rs` 中的 `HandRank`
  枚举设计精妙，它通过变体的顺序（从皇家同花顺到高牌）以及内部存储的比较信息（如对子的大小、踢脚牌），可以直接利用 `Ord`
  特性来比较两手牌的大小，代码既简洁又高效。

- **`distribute_pots` 函数**: `poker_eden_core/src/logic.rs` 中的 `distribute_pots`
  函数实现了健壮的边池分配逻辑。它通过逐级处理不同玩家的下注额度，构建并分配主池和各个边池，确保了在多人、不同筹码深度的
  All-In 场景下奖金分配的正确性。

## 未来可以改进的方向

- **更完善的客户端**: 当前的终端客户端功能较为基础，可以开发一个图形化界面（如使用 `egui` 或 `bevy`）来提升用户体验。
- **断线重连**: 实现一个完整的断线重连机制，允许玩家在网络中断后重新加入游戏。
- **持久化**: 将游戏房间和玩家数据持久化到数据库中。
- **更丰富的游戏选项**: 增加更多自定义游戏选项，如游戏速度、买入限制等。

---
<p align="center">
  Copyright © 2025 Peilin Fan | Licensed under the <a href="LICENSE">GPL-3.0 License</a>
</p>
