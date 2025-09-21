use crate::card::{Card, HandRank};
use crate::state::{GamePhase, GameState, Player, PlayerAction, PlayerId};
use crate::RoomId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type PlayerSecret = Uuid;

// --- 客户端 -> 服务器 的消息 ---
// 这些是客户端可以发送给服务器的指令或动作。

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ClientMessage {
    // --- 房间管理消息 ---
    /// 客户端请求创建一个新房间
    CreateRoom { nickname: String },
    /// 客户端请求加入一个已存在的房间
    JoinRoom { room_id: RoomId, nickname: String },

    // --- 游戏内消息 ---
    /// 玩家设置自己的昵称
    SetNickname(String),
    /// 玩家选择一个座位坐下
    RequestSeat { seat_id: u8, stack: u32 },
    /// 玩家从座位上站起 (进入观战)
    LeaveSeat,
    /// 玩家请求开始新的一局游戏 (通常由房主或自动触发)
    StartHand,
    /// 玩家在轮到自己时执行的游戏动作
    PerformAction(PlayerAction),
    /// 获取自己的手牌
    GetMyHand,
}

// --- 服务器 -> 客户端 的消息 ---
// 这些是服务器在游戏状态改变后，广播给所有客户端的事件通知。

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerMessage {
    // --- 房间管理消息 ---
    /// 成功加入或创建房间后，服务器私密地发给该玩家
    RoomJoined {
        your_id: PlayerId,
        your_secret: PlayerSecret, // 用于断线重连的凭证
        game_state: GameState, // 净化后的初始游戏状态
        host_id: PlayerId, // 房主ID
    },

    // --- 游戏状态更新消息 ---
    /// 完整游戏状态的快照。
    /// 通常在玩家刚加入房间或需要强制同步状态时发送。
    /// 发送时会调用 state.for_client(client_id) 来隐藏敏感信息。
    GameStateSnapshot(GameState),

    /// 一个新玩家加入了房间
    PlayerJoined { player: Player },

    /// 一个玩家离开了房间
    PlayerLeft { player_id: PlayerId },

    /// 一个玩家的状态更新了（例如：昵称，筹码，离线状态等）
    PlayerUpdated { player: Player },

    /// 新的一局开始
    HandStarted {
        /// 本局参与玩家的顺序
        hand_player_order: Vec<PlayerId>,
        /// 庄家(按钮)位置的玩家ID
        dealer_id: PlayerId,
    },

    /// 玩家执行了一个动作
    PlayerActed {
        player_id: PlayerId,
        action: PlayerAction,
        /// 执行动作后，该玩家在本轮的总下注额
        total_bet_this_round: u32,
        /// 执行动作后，该玩家剩余的筹码
        new_stack: u32,
        /// 执行动作后，总奖池金额
        new_pot: u32,
    },

    /// 轮到下一个玩家行动
    NextToAct {
        player_id: PlayerId,
        valid_actions: Vec<PlayerActionType>, // 新增：告诉客户端哪些动作是合法的
    },

    /// 发出公共牌 (翻牌、转牌、河牌)
    CommunityCardsDealt {
        phase: GamePhase, // Flop, Turn, or River
        cards: Vec<Card>,
    },

    /// 返还未被跟注的筹码
    BetReturned {
        player_id: PlayerId,
        amount: u32,
        new_stack: u32,
    },

    /// 摊牌阶段，公布结果
    Showdown {
        results: Vec<ShowdownResult>,
    },

    /// 玩家的手牌
    PlayerHand {
        hands: (Card, Card),
    },

    /// 服务器向特定客户端发送错误信息
    Info { message: String },
    Error { message: String },
}

/// 在 Showdown 消息中，用于描述单个玩家的结果
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ShowdownResult {
    pub player_id: PlayerId,
    /// 玩家的最终牌型
    pub hand_rank: Option<HandRank>,
    /// 玩家用于组成最佳牌型的底牌
    pub cards: Option<(Card, Card)>,
    /// 该玩家赢得的筹码数量
    pub winnings: u32,
}

// 用于告知客户端当前合法的动作类型，简化客户端UI逻辑
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum PlayerActionType {
    Fold,
    Check,
    Call(u32),   // 需要跟注的金额
    Bet(u32),    // 最小需要下注的金额
    Raise(u32),  // 最小需要加注的金额
}

impl From<PlayerAction> for ClientMessage {
    fn from(action: PlayerAction) -> Self {
        ClientMessage::PerformAction(action)
    }
}
