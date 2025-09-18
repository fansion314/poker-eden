use crate::card::*;
use crate::state::*;
use std::collections::HashMap;

// --- 核心游戏流程函数 ---
impl GameState {
    /// 开始新的一局游戏
    ///
    /// 这个函数负责初始化一局德州扑克所需的所有状态。
    /// - 重置奖池、公共牌等。
    /// - 为所有参与的玩家设置初始状态。
    /// - 创建一副新牌，洗牌，并给每个玩家发两张底牌。
    /// - 处理大小盲注。
    /// - 设置游戏阶段为 PreFlop，并确定第一个行动的玩家。
    ///
    /// # Panics
    /// 如果活跃玩家少于2人，则会 panic，因为游戏无法开始。
    pub fn start_new_hand(&mut self) {
        // 移除庄家轮换逻辑，由外部调用者负责
        // state.seated_players.rotate_left(1);

        // 1. 验证游戏开始的条件 (从轮换后的新顺序中过滤)
        self.hand_player_order = self
            .seated_players
            .iter()
            .filter(|id| self.players.get(id).map_or(false, |p| p.state != PlayerState::SittingOut && p.stack > 0))
            .cloned()
            .collect();

        let active_player_count = self.hand_player_order.len();
        if active_player_count < 2 {
            self.phase = GamePhase::WaitingForPlayers;
            return;
        }

        // 更新 PlayerId -> index 的映射
        self.player_indices = self.hand_player_order.iter().enumerate().map(|(i, id)| (*id, i)).collect();

        // 2. 重置游戏状态
        self.pot = 0;
        self.community_cards = vec![None; 5];
        self.cur_max_bet = 0;

        // 初始化基于Vec的结构
        self.player_cards = vec![(None, None); active_player_count];
        self.cur_bets = vec![0; active_player_count];
        // 初始化 player_has_acted 状态，所有人都未行动
        self.player_has_acted = vec![false; active_player_count];

        // 3. 创建和洗牌
        let total_cards_needed = active_player_count * 2 + 5;
        self.deck = generate_random_hand(total_cards_needed);

        // 4. 发底牌并设置玩家状态
        for (idx, player_id) in self.hand_player_order.iter().enumerate() {
            if let Some(player) = self.players.get_mut(player_id) {
                player.state = PlayerState::WaitingForTurn;
                let card1 = self.deck.pop().unwrap();
                let card2 = self.deck.pop().unwrap();
                self.player_cards[idx] = (Some(card1), Some(card2));
            }
        }

        // 5. MODIFIED: 处理盲注，增加两人单挑(Heads-up)的特殊逻辑
        let sb_idx;
        let bb_idx;
        let first_to_act_idx;

        if active_player_count == 2 {
            // 两人单挑规则:
            // - 庄家 (index 0) 是小盲, 翻牌前先行动
            // - 另一个玩家 (index 1) 是大盲
            sb_idx = 0;
            bb_idx = 1;
            first_to_act_idx = 0;
        } else {
            // 3人及以上规则:
            // - 庄家 (index 0)
            // - 小盲 (index 1)
            // - 大盲 (index 2)
            // - 枪口位 (大盲后，index 3) 先行动
            sb_idx = 1 % active_player_count;
            bb_idx = 2 % active_player_count;
            first_to_act_idx = (bb_idx + 1) % active_player_count;
        }

        let sb_id = self.hand_player_order[sb_idx];
        let sb_player = self.players.get_mut(&sb_id).unwrap();
        let sb_amount = self.small_blind.min(sb_player.stack);
        sb_player.stack -= sb_amount;
        self.pot += sb_amount;
        self.cur_bets[sb_idx] = sb_amount;
        if sb_player.stack == 0 { sb_player.state = PlayerState::AllIn; }

        let bb_id = self.hand_player_order[bb_idx];
        let bb_player = self.players.get_mut(&bb_id).unwrap();
        let bb_amount = self.big_blind.min(bb_player.stack);
        bb_player.stack -= bb_amount;
        self.pot += bb_amount;
        self.cur_bets[bb_idx] = bb_amount;
        if bb_player.stack == 0 { bb_player.state = PlayerState::AllIn; }

        self.cur_max_bet = self.big_blind;

        // 6. 设置游戏阶段和第一个行动者
        self.phase = GamePhase::PreFlop;
        self.cur_player_idx = Some(first_to_act_idx);

        let first_actor_id = self.hand_player_order[first_to_act_idx];
        if let Some(player) = self.players.get_mut(&first_actor_id) {
            player.state = PlayerState::Acting;
        }
    }

    /// 处理单个玩家的动作
    ///
    /// 这是游戏逻辑的核心驱动函数之一。它接收一个玩家的动作，
    /// 验证其合法性，然后更新游戏状态。
    /// - 扣除筹码，增加奖池。
    /// - 更改玩家状态 (如 Folded, AllIn)。
    /// - 更新当前轮的最大下注额。
    ///
    /// 在处理完动作后，它会检查当前下注轮是否结束。
    /// 如果是，则推进到下一个游戏阶段 (e.g., Flop -> Turn)。
    /// 如果否，则将行动权转移给下一个玩家。
    pub fn handle_player_action(&mut self, player_id: PlayerId, action: PlayerAction) {
        if self.current_player_id() != Some(player_id) { return; }

        let player_idx = *self.player_indices.get(&player_id).unwrap();
        let player_bet = self.cur_bets[player_idx];
        let amount_to_call = self.cur_max_bet - player_bet;

        {
            let player = self.players.get_mut(&player_id).unwrap();
            match action {
                PlayerAction::Fold => {
                    player.state = PlayerState::Folded;
                }
                PlayerAction::Check => {
                    // 必须是无人下注（或大盲注无人加注）时才能过牌
                    if amount_to_call != 0 { return; }
                }
                PlayerAction::Call => {
                    if amount_to_call > 0 {
                        let call_amount = amount_to_call.min(player.stack);
                        player.stack -= call_amount;
                        self.pot += call_amount;
                        self.cur_bets[player_idx] += call_amount;
                        if player.stack == 0 { player.state = PlayerState::AllIn; }
                    }
                }
                PlayerAction::BetOrRaise(amount) => {
                    // amount 是下注后的总额
                    let raise_amount = amount - player_bet;
                    // 基本条件: 总下注额 > 当前最高下注额，且下注的部分小于等于自己的总筹码
                    if amount <= self.cur_max_bet || raise_amount > player.stack { return; }

                    if amount_to_call == 0 { // 这是Bet
                        if amount < self.big_blind { return; } // Bet必须大于等于大盲
                    } else { // 这是Raise
                        // 加注额必须至少是上一轮下注/加注的差额
                        let min_raise_diff = self.cur_max_bet - player_bet;
                        if raise_amount < min_raise_diff && player.stack > raise_amount { return; }
                    }

                    player.stack -= raise_amount;
                    self.pot += raise_amount;
                    self.cur_bets[player_idx] = amount;
                    self.cur_max_bet = amount;
                    if player.stack == 0 { player.state = PlayerState::AllIn; }

                    // 当有人加注时，其他所有未弃牌的玩家都需要重新行动一轮。
                    // 我们通过重置他们的 `player_has_acted` 状态来实现这一点。
                    for (i, p_id) in self.hand_player_order.iter().enumerate() {
                        if let Some(p) = self.players.get(p_id) {
                            if p.id != player_id && p.state != PlayerState::Folded && p.state != PlayerState::AllIn {
                                self.player_has_acted[i] = false;
                            }
                        }
                    }
                }
            }
        }

        // NOTE: 无论玩家做什么动作，他都在本轮“表态”了。
        self.player_has_acted[player_idx] = true;

        // 检查是否只剩一人未弃牌
        let players_in_hand: Vec<_> = self.hand_player_order.iter().filter(|id| self.players.get(id).map_or(false, |p| p.state != PlayerState::Folded)).cloned().collect();
        if players_in_hand.len() <= 1 {
            // 如果是，直接分配底池，结束这局
            self.distribute_pot(players_in_hand);
            self.phase = GamePhase::Showdown;
            self.cur_player_idx = None;
            return;
        }

        if self.check_betting_round_over() {
            self.advance_to_next_phase();
        } else {
            self.advance_to_next_player();
        }
    }

    // --- 辅助逻辑函数 ---

    /// 将行动权转移给下一位合法的玩家
    fn advance_to_next_player(&mut self) {
        if self.cur_player_idx.is_none() { return; }
        let mut current_idx = self.cur_player_idx.unwrap();

        let current_player_id = self.hand_player_order[current_idx];
        if let Some(player) = self.players.get_mut(&current_player_id) {
            // 只有还在 Acting 状态的玩家需要变更为 WaitingForTurn
            if player.state == PlayerState::Acting {
                player.state = PlayerState::WaitingForTurn;
            }
        }

        // 循环查找下一个可以行动的玩家
        loop {
            current_idx = (current_idx + 1) % self.hand_player_order.len();
            let next_player_id = self.hand_player_order[current_idx];
            if let Some(player) = self.players.get(&next_player_id) {
                // 只有处于 WaitingForTurn 或 Acting (例如，重新轮到大盲)的玩家才能行动
                if matches!(player.state, PlayerState::WaitingForTurn | PlayerState::Acting) {
                    self.cur_player_idx = Some(current_idx);
                    self.players.get_mut(&next_player_id).unwrap().state = PlayerState::Acting;
                    return;
                }
            }
        }
    }

    /// 检查当前下注轮是否结束
    ///
    /// 下注轮结束的条件是:
    /// 1. 所有未弃牌 (Folded) 且未全下 (All-In) 的玩家，都已经在这一轮行动过 (player_has_acted == true)。
    /// 2. 并且，他们所有人的当前下注额 (cur_bets) 都等于当前轮的最高下注额 (cur_max_bet)。
    ///
    /// 这个逻辑正确地处理了:
    /// - 翻牌前大盲注的 "选择权" (Option): 如果前面玩家只是跟注，行动轮到大盲时，他的 `player_has_acted` 仍为 false，所以本轮不会结束，他可以选择过牌或加注。
    /// - 加注后重新开始一轮: 当有人加注，其他玩家的 `player_has_acted` 会被重置为 false，强迫他们必须再次行动。
    fn check_betting_round_over(&mut self) -> bool {
        let mut non_acting_players = 0; // 统计还能行动但未行动的玩家数

        for (idx, player_id) in self.hand_player_order.iter().enumerate() {
            if let Some(p) = self.players.get(player_id) {
                // 只关心那些还在牌局中且没有 all-in 的玩家
                if p.state != PlayerState::Folded && p.state != PlayerState::AllIn {
                    // 如果这位玩家的下注额不等于最高下注，那么本轮肯定没结束
                    if self.cur_bets[idx] != self.cur_max_bet {
                        return false;
                    }
                    // 如果这位玩家还没有行动过，那么本轮也还没结束
                    if !self.player_has_acted[idx] {
                        non_acting_players += 1;
                    }
                }
            }
        }

        // 如果所有需要表态的玩家的下注额都相等了，并且他们都已经行动过了 (non_acting_players == 0)，
        // 那么这一轮就结束了。
        non_acting_players == 0
    }


    /// 推进到下一个游戏阶段
    ///
    /// 在一轮下注结束后调用。
    /// - 根据当前阶段，发出公共牌 (Flop, Turn, River)。
    /// - 重置新一轮的下注状态。
    /// - 确定下一轮第一个行动的玩家 (通常是庄家左边的第一个未弃牌玩家)。
    /// - 如果已是 River 结束，则进入 Showdown (摊牌)阶段。
    fn advance_to_next_phase(&mut self) {
        self.cur_bets.iter_mut().for_each(|bet| *bet = 0);
        self.cur_max_bet = 0;
        self.cur_player_idx = None;

        // 为新一轮下注重置所有玩家的行动状态
        self.player_has_acted.fill(false);

        // 根据当前阶段推进
        match self.phase {
            GamePhase::PreFlop => {
                self.phase = GamePhase::Flop;
                self.community_cards[0] = self.deck.pop();
                self.community_cards[1] = self.deck.pop();
                self.community_cards[2] = self.deck.pop();
            }
            GamePhase::Flop => {
                self.phase = GamePhase::Turn;
                self.community_cards[3] = self.deck.pop();
            }
            GamePhase::Turn => {
                self.phase = GamePhase::River;
                self.community_cards[4] = self.deck.pop();
            }
            GamePhase::River => {
                self.phase = GamePhase::Showdown;
                self.handle_showdown();
                return;
            }
            _ => return, // 其他阶段不应调用此函数
        }

        // 确定下一轮有多少玩家可以行动 (未弃牌且未全下)
        let potential_actors: Vec<usize> = (1..self.hand_player_order.len())
            .chain(0..1)
            .filter(|&i| {
                let player_id = self.hand_player_order[i];
                self.players.get(&player_id).map_or(false, |p| {
                    !matches!(p.state, PlayerState::Folded | PlayerState::AllIn)
                })
            })
            .collect();

        // 如果可以行动的玩家少于2人（0或1），则没有更多下注轮，直接发完所有公共牌进入摊牌
        if potential_actors.len() < 2 {
            while let Some(pos) = self.community_cards.iter().position(|c| c.is_none()) {
                if let Some(card) = self.deck.pop() {
                    self.community_cards[pos] = Some(card);
                } else {
                    break; // 牌堆没牌了
                }
            }
            self.phase = GamePhase::Showdown;
            self.handle_showdown();
        } else {
            // 否则，正常开始下一轮，设置第一个可以行动的玩家
            let first_actor_idx = potential_actors[0];
            self.cur_player_idx = Some(first_actor_idx);
            let player_id = self.hand_player_order[first_actor_idx];
            self.players.get_mut(&player_id).unwrap().state = PlayerState::Acting;
        }
    }

    /// 处理摊牌逻辑
    ///
    /// - 找出所有未弃牌的玩家。
    /// - 为每个玩家评估他们能组成的最大手牌。
    /// - 比较牌力，找到一个或多个赢家。
    /// - 分配奖池。
    fn handle_showdown(&mut self) {
        self.cur_bets.iter_mut().for_each(|bet| *bet = 0);

        let mut best_rank: Option<HandRank> = None;
        let mut winners = Vec::new();
        let mut player_hands: HashMap<PlayerId, HandRank> = HashMap::new();
        let revealed_community_cards: Vec<Card> = self.community_cards.iter().flatten().cloned().collect();

        for (idx, player_id) in self.hand_player_order.iter().enumerate() {
            let player_state = &self.players.get(player_id).unwrap().state;
            if matches!(player_state, PlayerState::WaitingForTurn | PlayerState::Acting | PlayerState::AllIn) {
                if let (Some(card1), Some(card2)) = self.player_cards[idx] {
                    let mut all_cards = revealed_community_cards.clone();
                    all_cards.push(card1);
                    all_cards.push(card2);
                    let best_hand = find_best_hand(&all_cards);
                    player_hands.insert(*player_id, best_hand);
                }
            }
        }

        for (_, rank) in &player_hands {
            if best_rank.as_ref().map_or(true, |br| rank > br) {
                best_rank = Some(rank.clone());
            }
        }

        if let Some(br) = best_rank {
            for (player_id, rank) in player_hands {
                if rank == br { winners.push(player_id); }
            }
        }

        self.distribute_pot(winners);
        self.cur_player_idx = None;
    }

    /// 将奖池分配给赢家
    /// NOTE: 当前是简易实现，未处理边池(Side Pot)情况。
    fn distribute_pot(&mut self, winners: Vec<PlayerId>) {
        if winners.is_empty() { return; }

        let win_amount_per_player = self.pot / winners.len() as u32;
        let remainder = self.pot % winners.len() as u32;

        for (i, winner_id) in winners.iter().enumerate() {
            if let Some(player) = self.players.get_mut(winner_id) {
                player.stack += win_amount_per_player + if i == 0 { remainder } else { 0 };
                player.wins += 1;
            }
        }
        self.pot = 0;
    }
}
// --- 单元测试 ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};
    use crate::state::Player;
    use std::collections::VecDeque;
    use uuid::Uuid;

    // 辅助函数：创建用于测试的GameState
    fn setup_test_game(player_stacks: &[u32]) -> (GameState, Vec<PlayerId>) {
        let mut players = HashMap::new();
        let mut seated_players = VecDeque::new();
        let mut player_ids = Vec::new();

        for &stack in player_stacks {
            let player_id = Uuid::new_v4();
            let player = Player {
                id: player_id,
                nickname: format!("Player_{}", player_id.simple()),
                stack,
                wins: 0,
                losses: 0,
                state: PlayerState::WaitingForHand,
                seat_id: Some(players.len() as u8),
            };
            players.insert(player_id, player);
            seated_players.push_back(player_id);
            player_ids.push(player_id);
        }

        let state = GameState {
            players,
            seated_players,
            small_blind: 10,
            big_blind: 20,
            ..Default::default()
        };

        (state, player_ids)
    }

    #[test]
    fn test_start_new_hand_normal() {
        // 测试正常情况下的开局
        let (mut state, _p_ids) = setup_test_game(&[1000, 1000, 1000, 1000]);
        state.start_new_hand();

        // 验证玩家顺序和数量
        assert_eq!(state.hand_player_order.len(), 4);

        // 验证盲注
        let sb_id = state.hand_player_order[1];
        let bb_id = state.hand_player_order[2];
        assert_eq!(state.players.get(&sb_id).unwrap().stack, 990);
        assert_eq!(state.players.get(&bb_id).unwrap().stack, 980);
        assert_eq!(state.pot, 30);
        assert_eq!(state.cur_max_bet, 20);

        // 验证第一个行动者 (大盲注之后)
        let first_actor_idx = state.cur_player_idx.unwrap();
        assert_eq!(first_actor_idx, 3);
        let first_actor_id = state.hand_player_order[first_actor_idx];
        assert_eq!(state.players.get(&first_actor_id).unwrap().state, PlayerState::Acting);
    }

    #[test]
    fn test_player_action_fold_and_win() {
        // 测试玩家弃牌和最终一人获胜
        let (mut state, p_ids) = setup_test_game(&[1000, 1000, 1000]);
        state.start_new_hand(); // p0=庄家, p1=SB, p2=BB. 轮到p0行动

        let p0_id = state.hand_player_order[0];
        let p1_id = state.hand_player_order[1];

        // p0行动 (第一个行动者是p0)
        // Note: 3人局，BB(p2)之后是Dealer(p0)行动
        state.cur_player_idx = Some(0);
        state.handle_player_action(p0_id, PlayerAction::Fold);
        assert_eq!(state.players.get(&p0_id).unwrap().state, PlayerState::Folded);

        // p1行动
        state.handle_player_action(p1_id, PlayerAction::Fold);
        assert_eq!(state.players.get(&p1_id).unwrap().state, PlayerState::Folded);

        // 现在只剩p2，游戏应该结束，p2赢得盲注
        assert_eq!(state.phase, GamePhase::Showdown);
        // p2赢回自己的20大盲 + p1的10小盲
        assert_eq!(state.players.get(&p_ids[2]).unwrap().stack, 1000 - 20 + 30);
    }

    #[test]
    fn test_betting_round_ends_and_advances_to_flop() {
        // 测试一轮下注结束并进入Flop阶段
        let (mut state, _p_ids) = setup_test_game(&[1000, 1000, 1000]);
        state.start_new_hand(); // p0=D, p1=SB, p2=BB. 轮到p0行动.

        let p0_id = state.hand_player_order[0];
        let p1_id = state.hand_player_order[1];
        let p2_id = state.hand_player_order[2];

        // 3人局，行动顺序是 p0 -> p1 -> p2
        assert_eq!(state.cur_player_idx, Some(0));
        state.handle_player_action(p0_id, PlayerAction::Call); // p0跟20
        state.handle_player_action(p1_id, PlayerAction::Call); // p1补10
        state.handle_player_action(p2_id, PlayerAction::Check); // p2过牌

        // 验证阶段推进
        assert_eq!(state.phase, GamePhase::Flop);
        assert_eq!(state.pot, 60);
        assert_eq!(state.community_cards.iter().flatten().count(), 3);

        // 验证下注状态重置
        assert_eq!(state.cur_max_bet, 0);
        assert!(state.cur_bets.iter().all(|&b| b == 0));

        // 验证Flop轮第一个行动者是SB (如果还在牌局中)
        assert_eq!(state.current_player_id(), Some(p1_id));
    }

    #[test]
    fn test_showdown_logic_simple_winner() {
        // 测试摊牌逻辑
        let (mut state, p_ids) = setup_test_game(&[1000, 1000]);
        state.start_new_hand();

        let p0_id = p_ids[0]; // Dealer / SB
        let p1_id = p_ids[1]; // BB

        // Pre-flop action: p0 calls, p1 checks
        state.handle_player_action(p0_id, PlayerAction::Call);
        state.handle_player_action(p1_id, PlayerAction::Check);

        // Manually set phase and cards for showdown
        state.phase = GamePhase::Showdown;
        assert_eq!(state.pot, 40); // SB 20 + BB 20
        assert_eq!(state.cur_bets, vec![0, 0]);

        state.community_cards = vec![
            Some(Card::new(Rank::Ace, Suit::Spade)),
            Some(Card::new(Rank::King, Suit::Spade)),
            Some(Card::new(Rank::Queen, Suit::Spade)),
            Some(Card::new(Rank::Two, Suit::Heart)),
            Some(Card::new(Rank::Three, Suit::Heart)),
        ];
        // p0: 同花顺
        state.player_cards[0] = (
            Some(Card::new(Rank::Jack, Suit::Spade)),
            Some(Card::new(Rank::Ten, Suit::Spade)),
        );
        // p1: 三条A
        state.player_cards[1] = (
            Some(Card::new(Rank::Ace, Suit::Club)),
            Some(Card::new(Rank::Ace, Suit::Diamond)),
        );

        state.players.get_mut(&p0_id).unwrap().state = PlayerState::WaitingForTurn;
        state.players.get_mut(&p1_id).unwrap().state = PlayerState::WaitingForTurn;

        state.handle_showdown();

        assert_eq!(state.phase, GamePhase::Showdown);
        let p0_stack = state.players.get(&p0_id).unwrap().stack;
        let p1_stack = state.players.get(&p1_id).unwrap().stack;

        // p0 是庄家/小盲, p1 是大盲. Pre-flop两人都投入了20.
        // p0赢得了40的底池. 初始1000, 投入20, 赢得40, 最终 1020.
        assert_eq!(p0_stack, 1000 - 20 + 40);
        // p1输了. 初始1000, 投入20, 最终 980.
        assert_eq!(p1_stack, 1000 - 20);
    }

    #[test]
    fn test_start_new_hand_heads_up_rules() {
        // 测试两人单挑(Heads-up)的特殊规则
        let (mut state, p_ids) = setup_test_game(&[1000, 1000]);
        state.start_new_hand();

        let dealer_id = p_ids[0]; // 庄家
        let bb_id = p_ids[1];     // 大盲

        // 庄家(p0)是小盲
        assert_eq!(state.players.get(&dealer_id).unwrap().stack, 990);
        // p1是大盲
        assert_eq!(state.players.get(&bb_id).unwrap().stack, 980);

        // 翻牌前，庄家(p0)先行动
        assert_eq!(state.cur_player_idx, Some(0));
        assert_eq!(state.current_player_id(), Some(dealer_id));

        // 庄家跟注
        state.handle_player_action(dealer_id, PlayerAction::Call);
        // 轮到大盲行动
        assert_eq!(state.current_player_id(), Some(bb_id));

        // 大盲过牌，进入翻牌圈
        state.handle_player_action(bb_id, PlayerAction::Check);
        assert_eq!(state.phase, GamePhase::Flop);

        // 翻牌后，大盲(p1)先行动
        assert_eq!(state.current_player_id(), Some(bb_id));
    }

    #[test]
    fn test_walk_bb_wins_blinds() {
        // 测试所有人都弃牌，大盲直接获胜 (Walk)
        let (mut state, p_ids) = setup_test_game(&[1000, 1000, 1000]);
        state.start_new_hand(); // p0=D, p1=SB, p2=BB

        let p0_id = p_ids[0];
        let p1_id = p_ids[1];
        let p2_id = p_ids[2];

        // 行动顺序 p0 -> p1 -> p2
        state.cur_player_idx = Some(0);
        state.handle_player_action(p0_id, PlayerAction::Fold);
        state.handle_player_action(p1_id, PlayerAction::Fold);

        // 此时只剩大盲，牌局应结束
        assert_eq!(state.phase, GamePhase::Showdown);
        // 大盲拿回自己的20，并赢得小盲的10
        assert_eq!(state.players.get(&p2_id).unwrap().stack, 1000 - 20 + 30);
        assert_eq!(state.pot, 0);
    }

    #[test]
    fn test_full_betting_round_with_raise_and_reraise() {
        // 测试包含加注和再加注的完整下注轮
        let (mut state, p_ids) = setup_test_game(&[1000, 1000, 1000, 1000]);
        state.start_new_hand(); // p0=D, p1=SB, p2=BB, p3=UTG

        let p1_id = p_ids[1]; // SB
        let p2_id = p_ids[2]; // BB
        let p3_id = p_ids[3]; // UTG
        let p0_id = p_ids[0]; // D

        // p3 (UTG) 加注到 60
        state.handle_player_action(p3_id, PlayerAction::BetOrRaise(60));
        assert_eq!(state.cur_max_bet, 60);
        assert_eq!(state.players.get(&p3_id).unwrap().stack, 940);

        // p0 (Dealer) 跟注 60
        state.handle_player_action(p0_id, PlayerAction::Call);
        assert_eq!(state.players.get(&p0_id).unwrap().stack, 940);

        // p1 (SB) 再加注到 180
        state.handle_player_action(p1_id, PlayerAction::BetOrRaise(180));
        assert_eq!(state.cur_max_bet, 180);
        assert_eq!(state.players.get(&p1_id).unwrap().stack, 820); // 1000 - 180

        // p2 (BB) 弃牌
        state.handle_player_action(p2_id, PlayerAction::Fold);
        assert_eq!(state.players.get(&p2_id).unwrap().state, PlayerState::Folded);

        // 轮回到 p3，他需要补齐差额 (180 - 60 = 120)
        assert_eq!(state.current_player_id(), Some(p3_id));
        state.handle_player_action(p3_id, PlayerAction::Call);
        assert_eq!(state.players.get(&p3_id).unwrap().stack, 940 - 120);

        // 轮回到 p0，他也需要补齐差额 (180 - 60 = 120)
        assert_eq!(state.current_player_id(), Some(p0_id));
        state.handle_player_action(p0_id, PlayerAction::Call);
        assert_eq!(state.players.get(&p0_id).unwrap().stack, 940 - 120);

        // p1 是最后一个加注者，他之后所有人都跟注了，下注轮结束
        assert_eq!(state.phase, GamePhase::Flop);
        // Pot: SB(180) + BB(20) + UTG(180) + D(180) = 560
        assert_eq!(state.pot, 180 + 20 + 180 + 180);
    }

    #[test]
    fn test_player_all_in_on_blind() {
        // 测试玩家在下盲注时就All-in
        let (mut state, p_ids) = setup_test_game(&[1000, 15, 1000]); // p1 只有 15
        state.start_new_hand(); // p0=D, p1=SB, p2=BB

        let p1_id = p_ids[1];
        // p1 下小盲注10，还剩5
        assert_eq!(state.players.get(&p1_id).unwrap().stack, 5);

        // 轮到p0行动，他跟注20
        let p0_id = p_ids[0];
        state.handle_player_action(p0_id, PlayerAction::Call);

        // 轮到p1行动，他跟注剩下的5，All-in
        state.handle_player_action(p1_id, PlayerAction::Call);
        assert_eq!(state.players.get(&p1_id).unwrap().stack, 0);
        assert_eq!(state.players.get(&p1_id).unwrap().state, PlayerState::AllIn);
        assert_eq!(state.cur_bets[1], 15); // SB 10 + Call 5
    }

    #[test]
    fn test_showdown_split_pot() {
        // 测试摊牌平局，平分底池
        let (mut state, _p_ids) = setup_test_game(&[1000, 1000]);
        state.start_new_hand();
        state.phase = GamePhase::Showdown;
        state.pot = 200;

        // p0 和 p1 的牌，加上公共牌，构成完全相同的牌型 (A-K-Q-J-T顺子)
        state.community_cards = vec![
            Some(Card::new(Rank::Ace, Suit::Spade)),
            Some(Card::new(Rank::King, Suit::Heart)),
            Some(Card::new(Rank::Queen, Suit::Diamond)),
            Some(Card::new(Rank::Jack, Suit::Club)),
            Some(Card::new(Rank::Two, Suit::Heart)),
        ];
        state.player_cards[0] = (Some(Card::new(Rank::Ten, Suit::Spade)), Some(Card::new(Rank::Nine, Suit::Spade)));
        state.player_cards[1] = (Some(Card::new(Rank::Ten, Suit::Heart)), Some(Card::new(Rank::Nine, Suit::Heart)));

        let p0_id = state.hand_player_order[0];
        let p1_id = state.hand_player_order[1];
        state.players.get_mut(&p0_id).unwrap().state = PlayerState::WaitingForTurn;
        state.players.get_mut(&p1_id).unwrap().state = PlayerState::WaitingForTurn;

        state.handle_showdown();

        assert_eq!(state.phase, GamePhase::Showdown);
        // 初始都是1000，减去盲注，然后平分200底池，每人拿回100
        // p0是SB，投入10，拿回100
        assert_eq!(state.players.get(&p0_id).unwrap().stack, 1000 - 10 + 100);
        // p1是BB，投入20，拿回100
        assert_eq!(state.players.get(&p1_id).unwrap().stack, 1000 - 20 + 100);
    }

    #[test]
    fn test_multiple_all_ins_auto_showdown() {
        // 测试多于一个玩家All-in，游戏自动发完牌并进入摊牌
        let (mut state, p_ids) = setup_test_game(&[50, 100, 1000]); // p0, p1 筹码较少
        state.start_new_hand(); // p0=D, p1=SB, p2=BB

        let p0_id = p_ids[0];
        let p1_id = p_ids[1];
        let p2_id = p_ids[2];

        // p0 (D) all-in 50
        state.handle_player_action(p0_id, PlayerAction::BetOrRaise(50));
        assert_eq!(state.players.get(&p0_id).unwrap().state, PlayerState::AllIn);

        // p1 (SB) all-in 100
        state.handle_player_action(p1_id, PlayerAction::BetOrRaise(100));
        assert_eq!(state.players.get(&p1_id).unwrap().state, PlayerState::AllIn);

        // p2 (BB) call 100
        state.handle_player_action(p2_id, PlayerAction::Call);

        // 因为除了p2之外所有人都all-in了，没有后续下注轮
        // 游戏应该直接发完所有公共牌并进入摊牌
        assert_eq!(state.phase, GamePhase::Showdown);
        assert_eq!(state.community_cards.iter().all(|c| c.is_some()), true);
        assert_eq!(state.community_cards.iter().flatten().count(), 5);
    }

    #[test]
    fn test_full_multi_stage_hand_flow() {
        // 测试一个完整的多人、多阶段牌局流程
        let (mut state, p_ids) = setup_test_game(&[2000, 2000, 2000, 2000]);
        state.start_new_hand();

        let p0_id = p_ids[0]; // D
        let p1_id = p_ids[1]; // SB
        let p2_id = p_ids[2]; // BB
        let p3_id = p_ids[3]; // UTG

        // --- 翻牌前 (Pre-flop) ---
        // UTG 加注到 60
        state.handle_player_action(p3_id, PlayerAction::BetOrRaise(60));
        // D 弃牌
        state.handle_player_action(p0_id, PlayerAction::Fold);
        // SB 跟注 (补50)
        state.handle_player_action(p1_id, PlayerAction::Call);
        // BB 跟注 (补40)
        state.handle_player_action(p2_id, PlayerAction::Call);

        assert_eq!(state.phase, GamePhase::Flop);
        assert_eq!(state.pot, 180); // 60 * 3
        assert_eq!(state.current_player_id(), Some(p1_id)); // Flop轮到SB先行动

        // --- 翻牌圈 (Flop) ---
        // SB 过牌
        state.handle_player_action(p1_id, PlayerAction::Check);
        // BB 过牌
        state.handle_player_action(p2_id, PlayerAction::Check);
        // UTG 下注 90
        state.handle_player_action(p3_id, PlayerAction::BetOrRaise(90));
        // SB 弃牌
        state.handle_player_action(p1_id, PlayerAction::Fold);
        // BB 跟注 90
        state.handle_player_action(p2_id, PlayerAction::Call);

        assert_eq!(state.phase, GamePhase::Turn);
        assert_eq!(state.pot, 180 + 90 + 90); // 360
        assert_eq!(state.current_player_id(), Some(p2_id)); // Turn轮到BB先行动

        // --- 转牌圈 (Turn) ---
        // BB 过牌
        state.handle_player_action(p2_id, PlayerAction::Check);
        // UTG 过牌
        state.handle_player_action(p3_id, PlayerAction::Check);

        assert_eq!(state.phase, GamePhase::River);
        assert_eq!(state.pot, 360);
        assert_eq!(state.current_player_id(), Some(p2_id)); // River轮到BB先行动

        // --- 摊牌 (Showdown) ---
        // 手动设置牌面，让 p2 获胜
        state.community_cards = vec![
            Some(Card::new(Rank::Ace, Suit::Spade)),
            Some(Card::new(Rank::King, Suit::Spade)),
            Some(Card::new(Rank::Two, Suit::Heart)),
            Some(Card::new(Rank::Three, Suit::Heart)),
            Some(Card::new(Rank::Four, Suit::Club)),
        ];
        // p2 (BB): 一对A
        state.player_cards[2] = (Some(Card::new(Rank::Ace, Suit::Club)), Some(Card::new(Rank::Queen, Suit::Diamond)));
        // p3 (UTG): 一对K
        state.player_cards[3] = (Some(Card::new(Rank::King, Suit::Club)), Some(Card::new(Rank::Queen, Suit::Spade)));

        // --- 河牌圈 (River) ---
        // BB 下注 200
        state.handle_player_action(p2_id, PlayerAction::BetOrRaise(200));
        // UTG 跟注 200
        state.handle_player_action(p3_id, PlayerAction::Call);

        assert_eq!(state.phase, GamePhase::Showdown);
        let p2_final_stack = state.players.get(&p2_id).unwrap().stack;
        let p3_final_stack = state.players.get(&p3_id).unwrap().stack;

        // p2 总共投入: 60 (pre) + 90 (flop) + 200 (river) = 350
        // p3 总共投入: 60 (pre) + 90 (flop) + 200 (river) = 350
        // p2 赢了 760 的底池
        assert_eq!(p2_final_stack, 2000 - 350 + 760);
        assert_eq!(p3_final_stack, 2000 - 350);
    }
}
