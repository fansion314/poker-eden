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
    // 轮换的、包含所有就座玩家的列表。每局开始时轮换。
    pub seated_players: VecDeque<PlayerId>,
    // 当前牌局的玩家顺序，不包含观战者
    pub hand_player_order: Vec<PlayerId>,
    // 方便通过PlayerId快速查找其在hand_player_order中的索引
    #[serde(skip)]
    pub player_indices: HashMap<PlayerId, usize>,

    pub phase: GamePhase,
    pub pot: u32,  // 总奖池金额
    // 对于服务端，此向量在内存中。
    // 对于客户端，这里长度为5，未翻开的牌是None。
    pub community_cards: Vec<Option<Card>>,
    // 服务端持有的完整牌堆，不会发给客户端。
    #[serde(skip)] // 确保deck不会被序列化发给客户端
    pub deck: Vec<Card>,

    // 服务端存有所有玩家的真实底牌 (Some(c1), Some(c2))
    // 客户端只知道自己的真实底牌，其他玩家的底牌为 (None, None)
    // 玩家手牌，其索引对应 hand_player_order 中的索引
    pub player_cards: Vec<(Option<Card>, Option<Card>)>,
    // 当前轮下注额，其索引对应 hand_player_order 中的索引
    pub cur_bets: Vec<u32>,

    pub cur_player_idx: Option<usize>,  // 当前应该行动的玩家在 player_order 中的索引
    pub cur_max_bet: u32, // 当前轮下注的最高金额
    pub small_blind: u32, // 小盲注金额
    pub big_blind: u32, // 大盲注金额
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
    HandOver, // 一局结束，结算完成
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
    /// 在观众席 (Spectating)
    Spectating,
    /// 等待中 (Waiting)
    WaitingForHand,
    /// 轮到其行动 (Acting)
    Acting,
    /// 等待他人 (WaitingForTurn)
    WaitingForTurn,
    /// 已全下 (All-In)
    AllIn,
    /// 已弃牌 (Folded)
    Folded,
    /// 离席 (Sitting Out)
    SittingOut,
}

// --- GameState 的实现方法 ---

impl GameState {
    /// 获取当前行动的玩家ID (如果存在)
    pub fn current_player_id(&self) -> Option<PlayerId> {
        self.cur_player_idx.map(|idx| self.hand_player_order[idx])
    }

    pub fn get_players_in_hand(&self) -> Vec<PlayerId> {
        self.hand_player_order
            .iter()
            .filter(|id| {
                let player = self.players.get(id).unwrap();
                matches!(player.state, PlayerState::Acting | PlayerState::WaitingForTurn | PlayerState::AllIn)
            })
            .cloned()
            .collect()
    }

    pub fn for_client(&self, client_id: &PlayerId) -> Self {
        let mut client_state = self.clone();
        client_state.deck.clear();

        // 获取当前客户端在牌局中的索引
        let client_idx_opt = self.player_indices.get(client_id).copied();

        if self.phase == GamePhase::Showdown || self.phase == GamePhase::HandOver {
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

