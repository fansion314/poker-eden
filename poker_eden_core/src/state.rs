use crate::card::Card;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use uuid::Uuid;

pub type RoomId = Uuid;
pub type PlayerId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    // ！房间加入时同步的状态
    pub room_id: RoomId,
    pub players: HashMap<PlayerId, Player>,  // 可以根据player id查找player
    pub small_blind: u32, // 小盲注金额
    pub big_blind: u32, // 大盲注金额
    pub seats: u8, // 房间总座位数

    // ！本局开始时同步的状态
    // 轮换的、包含所有就座玩家的列表。每局开始时轮换。
    pub seated_players: VecDeque<PlayerId>,
    // 当前牌局的玩家顺序，不包含观战者
    pub hand_player_order: Vec<PlayerId>,
    // 方便通过PlayerId快速查找其在hand_player_order中的索引
    pub player_indices: HashMap<PlayerId, usize>,
    // 服务端持有的完整牌堆，不会发给客户端。
    #[serde(skip)] // 确保deck不会被序列化发给客户端
    pub(crate) deck: Vec<Card>,

    // ！游戏过程中随时同步的状态
    pub phase: GamePhase,
    // 总奖池金额
    pub pot: u32,
    // 每个玩家的总下注额，其索引对应 hand_player_order 中的索引
    pub bets: Vec<u32>,

    // 公共牌数组，长度为5。已发的牌是 Some(card)，未发的牌是 None
    pub community_cards: Vec<Option<Card>>,
    // 服务端存有所有玩家的真实底牌 (Some(c1), Some(c2))
    // 客户端只知道自己的真实底牌，其他玩家的底牌为 (None, None)
    // 玩家手牌，其索引对应 hand_player_order 中的索引
    pub player_cards: Vec<(Option<Card>, Option<Card>)>,

    // ！游戏中间变量
    // 在每轮下注开始时重置为 all false
    // 当玩家加注时，其他人的此状态会被重置为 false
    #[serde(skip)]
    pub(crate) player_has_acted: Vec<bool>,
    pub cur_player_idx: usize,  // 当前应该行动的玩家在 hand_player_order 中的索引
    pub max_bet: u32, // 下注的最高金额
    pub last_bet: u32, // 上轮最终下注金额
    pub last_raise_amount: u32,  // 最小加注额
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum GamePhase {
    WaitingForPlayers,
    PreFlop,
    Flop,
    Turn,
    River,
    Showdown, // 一局结束，结算完成
}

impl Display for GamePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GamePhase::WaitingForPlayers => write!(f, "等待玩家"),
            GamePhase::PreFlop => write!(f, "预发牌"),
            GamePhase::Flop => write!(f, "发牌"),
            GamePhase::Turn => write!(f, "转牌"),
            GamePhase::River => write!(f, "河牌"),
            GamePhase::Showdown => write!(f, "摊牌"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlayerAction {
    Check,     // 过牌
    Call,      // 跟注
    BetOrRaise(u32), // 下注或加注，金额为下注后的总额
    Fold,      // 弃牌
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlayerState {
    /// 等待新牌局: 已入座，等待下一局开始后发牌。
    Waiting,
    /// 游戏中
    Playing,
    /// 已全下 (All-In)
    AllIn,
    /// 已弃牌 (Folded)
    Folded,
    /// 离席 (Sitting Out): 离席，不参与游戏，但是可以观看游戏进行。
    SittingOut,
    /// 离线或离开
    Offline,
}

impl Display for PlayerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayerState::Waiting => write!(f, "等待"),
            PlayerState::Playing => write!(f, "游戏中"),
            PlayerState::AllIn => write!(f, "已全下"),
            PlayerState::Folded => write!(f, "已弃牌"),
            PlayerState::SittingOut => write!(f, "离席"),
            PlayerState::Offline => write!(f, "掉线"),
        }
    }
}

// --- GameState 的实现方法 ---

impl Default for GameState {
    fn default() -> Self {
        Self {
            room_id: RoomId::new_v4(),
            players: HashMap::new(),
            seated_players: VecDeque::new(),
            hand_player_order: vec![],
            player_indices: HashMap::new(),
            phase: GamePhase::WaitingForPlayers,
            pot: 0,
            community_cards: vec![None; 5],
            deck: vec![],
            player_cards: vec![(None, None); 5],
            bets: vec![],
            player_has_acted: vec![],
            cur_player_idx: 0,
            max_bet: 0,
            last_bet: 0,
            last_raise_amount: 0,
            small_blind: 100,
            big_blind: 200,
            seats: 10,
        }
    }
}

impl GameState {
    /// 获取当前行动的玩家ID (如果存在)
    pub fn current_player_id(&self) -> Option<PlayerId> {
        self.hand_player_order.get(self.cur_player_idx).copied()
    }

    pub fn get_players_in_hand(&self) -> Vec<PlayerId> {
        self.hand_player_order
            .iter()
            .filter(|id| {
                let player = self.players.get(id).unwrap();
                matches!(player.state, PlayerState::Playing | PlayerState::AllIn)
            })
            .cloned()
            .collect()
    }

    pub fn for_client(&self, client_id: &PlayerId) -> Self {
        let mut client_state = self.clone();
        client_state.deck.clear();

        // 获取当前客户端在牌局中的索引
        let client_idx_opt = self.player_indices.get(client_id).copied();

        if self.phase == GamePhase::Showdown {
            let players_in_hand_set: std::collections::HashSet<_> = self.get_players_in_hand().into_iter().collect();

            for (i, cards) in client_state.player_cards.iter_mut().enumerate() {
                let player_id = &self.hand_player_order[i];
                if !players_in_hand_set.contains(player_id) && Some(i) != client_idx_opt {
                    *cards = (None, None);
                }
            }
        } else {
            for (i, cards) in client_state.player_cards.iter_mut().enumerate() {
                if Some(i) != client_idx_opt {
                    *cards = (None, None);
                }
            }
        }

        client_state
    }
}

