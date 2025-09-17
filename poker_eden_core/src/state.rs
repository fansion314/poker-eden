use crate::card::Card;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;

pub type RoomId = Uuid;
pub type PlayerId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub room_id: RoomId,
    pub players: HashMap<PlayerId, Player>,  // 可以根据player id查找player
    pub player_order: VecDeque<PlayerId>,  // 玩家顺序，索引0的位置为庄家，后续玩家按顺时针顺序排列。每次结束后用rotate来切换庄家
    pub phase: GamePhase,
    pub pot: u32,  // 总金额，包括当前玩家的下注金额
    pub cards: Vec<Option<Card>>,  // 5张牌，还未生成的牌用None表示。对于服务端的GameState，在本局开局时就已全部生成，对于客户端，在游戏过程中可能会收到新的牌，未知的牌用None表示
    pub player_cards: HashMap<PlayerId, (Option<Card>, Option<Card>)>,  // 玩家的牌，同样对于服务端和客户端是不同的。
    pub cur_player: Option<usize>,  // 当前应该行动的玩家
    pub cur_max_pot: u32, // 当前轮最大的下注金额
    pub cur_pots: Vec<u32>,   // 当前轮每位玩家的下注金额（对应player_order）用于判断玩家是否还需要补足筹码
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub id: PlayerId,
    pub nickname: String,
    pub stack: u32,  // 剩余筹码
    pub wins: u32,  // 本次游戏赢的次数
    pub losses: u32,  // 本次游戏输光全部筹码的次数
    pub state: PlayerState,
    pub seat_id: Option<u8>,  // 座位号（总共若干座位）由用户自己选择座位
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GamePhase {
    WaitingForPlayers,
    PreFlop,
    Flop,
    Turn,
    River,
    Showdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlayerAction {
    Check,
    Bet(u32),
    Call,
    Raise(u32),
    Fold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlayerState {
    /// 在观众席 (Spectating)
    /// 玩家正在观战，未加入任何牌桌。
    Spectating,

    /// 等待中 (Waiting)
    /// 玩家已在牌桌就座，但正在等待下一局游戏开始。
    WaitingForHand,

    /// 游戏中 - 轮到其行动 (In-Game, Acting)
    /// 玩家在当前牌局中，并且轮到他/她做出决定（跟注、加注等）。
    /// 这对应您提到的“决定中”。
    Acting,

    /// 游戏中 - 等待他人 (In-Game, Waiting)
    /// 玩家在当前牌局中，但正在等待其他玩家行动。
    /// 这是“游戏中”最常见的状态。
    WaitingForTurn,

    /// 游戏中 - 已全下 (In-Game, All-In)
    /// 玩家在当前牌局中，并已投入所有筹码，无法再进行任何操作，只能等待摊牌。
    AllIn,

    /// 已弃牌 (Folded)
    /// 玩家在当前牌局中，但已经放弃了手牌。
    Folded,

    /// 离席 (Sitting Out)
    /// 玩家虽然还占据着座位，但暂时不参与游戏。会自动支付盲注并弃牌。
    SittingOut,

    /// 已离线 (Offline)
    /// 玩家连接已断开。
    Offline,
}
