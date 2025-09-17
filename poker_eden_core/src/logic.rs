use crate::card::*;
use crate::state::*;
use std::collections::HashMap;

// --- 核心游戏流程函数 ---

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
pub fn start_new_hand(state: &mut GameState) {
    // 5. 轮换庄家位置
    state.seated_players.rotate_left(1);

    // 1. 验证游戏开始的条件 (从轮换后的新顺序中过滤)
    // 4. 更新player_order为本局游玩的用户
    state.hand_player_order = state
        .seated_players
        .iter()
        .filter(|id| state.players.get(id).map_or(false, |p| p.state != PlayerState::SittingOut))
        .cloned()
        .collect();

    let active_player_count = state.hand_player_order.len();
    if active_player_count < 2 {
        state.phase = GamePhase::WaitingForPlayers;
        return;
    }

    // 更新 PlayerId -> index 的映射
    state.player_indices = state.hand_player_order.iter().enumerate().map(|(i, id)| (*id, i)).collect();

    // 2. 重置游戏状态
    state.pot = 0;
    state.community_cards = vec![None; 5];
    state.cur_max_bet = 0;

    // 初始化基于Vec的结构
    state.player_cards = vec![(None, None); active_player_count];
    state.cur_bets = vec![0; active_player_count];

    // 3. 创建和洗牌 (修正函数名)
    let total_cards_needed = active_player_count * 2 + 5;
    state.deck = generate_random_hand(total_cards_needed);

    // 4. 发底牌并设置玩家状态
    for (idx, player_id) in state.hand_player_order.iter().enumerate() {
        if let Some(player) = state.players.get_mut(player_id) {
            player.state = PlayerState::WaitingForTurn;
            let card1 = state.deck.pop().unwrap();
            let card2 = state.deck.pop().unwrap();
            state.player_cards[idx] = (Some(card1), Some(card2));
        }
    }

    // 5. 处理盲注 (逻辑不变，但实现方式改变)
    // 庄家是 hand_player_order[0]
    let sb_idx = 1 % active_player_count;
    let bb_idx = 2 % active_player_count;

    let sb_id = state.hand_player_order[sb_idx];
    let bb_id = state.hand_player_order[bb_idx];

    // 2. 避免同时可变借用，直接访问 state.players
    let sb_player = state.players.get_mut(&sb_id).unwrap();
    let sb_amount = state.small_blind.min(sb_player.stack);
    sb_player.stack -= sb_amount;
    state.pot += sb_amount;
    state.cur_bets[sb_idx] = sb_amount;
    // 3. 投入大小盲注后，加上判断是否已经无筹码的判断
    if sb_player.stack == 0 { sb_player.state = PlayerState::AllIn; }

    let bb_player = state.players.get_mut(&bb_id).unwrap();
    let bb_amount = state.big_blind.min(bb_player.stack);
    bb_player.stack -= bb_amount;
    state.pot += bb_amount;
    state.cur_bets[bb_idx] = bb_amount;
    if bb_player.stack == 0 { bb_player.state = PlayerState::AllIn; }

    state.cur_max_bet = state.big_blind;

    // 6. 设置游戏阶段和第一个行动者
    state.phase = GamePhase::PreFlop;
    let first_to_act_idx = (bb_idx + 1) % active_player_count;
    state.cur_player_idx = Some(first_to_act_idx);

    let first_actor_id = state.hand_player_order[first_to_act_idx];
    if let Some(player) = state.players.get_mut(&first_actor_id) {
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
pub fn handle_player_action(state: &mut GameState, player_id: PlayerId, action: PlayerAction) {
    if state.current_player_id() != Some(player_id) { return; }

    let player_idx = *state.player_indices.get(&player_id).unwrap();
    let player_bet = state.cur_bets[player_idx];
    let amount_to_call = state.cur_max_bet - player_bet;

    {
        let player = state.players.get_mut(&player_id).unwrap();
        match action {
            PlayerAction::Fold => {
                player.state = PlayerState::Folded;
            }
            PlayerAction::Check => {
                if amount_to_call != 0 { return; }
            }
            PlayerAction::Call => {
                if amount_to_call > 0 {
                    let call_amount = amount_to_call.min(player.stack);
                    player.stack -= call_amount;
                    state.pot += call_amount;
                    state.cur_bets[player_idx] += call_amount;
                    if player.stack == 0 { player.state = PlayerState::AllIn; }
                }
            }
            // 6. 合并Bet和Raise
            PlayerAction::BetOrRaise(amount) => {
                // amount 是下注后的总额
                let raise_amount = amount - player_bet;
                // 基本条件: 总下注额 > 当前最高下注额，且小于等于自己的总筹码
                if amount <= state.cur_max_bet || amount > player.stack + player_bet { return; }

                if amount_to_call == 0 { // 这是Bet
                    if amount < state.big_blind { return; } // Bet必须大于等于大盲
                } else { // 这是Raise
                    let min_raise_amount = state.cur_max_bet - player_bet;
                    if raise_amount < min_raise_amount { return; } // Raise额度必须至少是上一个bet/raise的额度
                }

                player.stack -= raise_amount;
                state.pot += raise_amount;
                state.cur_bets[player_idx] = amount;
                state.cur_max_bet = amount;
                if player.stack == 0 { player.state = PlayerState::AllIn; }
            }
        }
    }

    let players_in_hand: Vec<_> = state.hand_player_order.iter().filter(|id| state.players.get(id).map_or(false, |p| p.state != PlayerState::Folded)).cloned().collect();
    if players_in_hand.len() <= 1 {
        distribute_pot(state, players_in_hand);
        state.phase = GamePhase::HandOver;
        return;
    }

    if check_betting_round_over(state) {
        advance_to_next_phase(state);
    } else {
        advance_to_next_player(state);
    }
}

// --- 辅助逻辑函数 ---

/// 将行动权转移给下一位合法的玩家
fn advance_to_next_player(state: &mut GameState) {
    let mut current_idx = state.cur_player_idx.unwrap();

    let current_player_id = state.hand_player_order[current_idx];
    if let Some(player) = state.players.get_mut(&current_player_id) {
        if player.state == PlayerState::Acting { player.state = PlayerState::WaitingForTurn; }
    }

    loop {
        current_idx = (current_idx + 1) % state.hand_player_order.len();
        let next_player_id = state.hand_player_order[current_idx];
        if let Some(player) = state.players.get(&next_player_id) {
            if matches!(player.state, PlayerState::WaitingForTurn | PlayerState::Acting) {
                state.cur_player_idx = Some(current_idx);
                state.players.get_mut(&next_player_id).unwrap().state = PlayerState::Acting;
                return;
            }
        }
    }
}

/// 检查当前下注轮是否结束
fn check_betting_round_over(state: &GameState) -> bool {
    let mut all_acted = true;
    for (idx, player_id) in state.hand_player_order.iter().enumerate() {
        if let Some(p) = state.players.get(player_id) {
            if matches!(p.state, PlayerState::Acting | PlayerState::WaitingForTurn) {
                if state.cur_bets[idx] != state.cur_max_bet {
                    all_acted = false;
                    break;
                }
            }
        }
    }

    // 如果所有人都all-in，也算结束
    let can_act_count = state.hand_player_order.iter().filter(|id| {
        let p = state.players.get(id).unwrap();
        matches!(p.state, PlayerState::Acting | PlayerState::WaitingForTurn)
    }).count();

    all_acted || can_act_count == 0
}

/// 推进到下一个游戏阶段
///
/// 在一轮下注结束后调用。
/// - 根据当前阶段，发出公共牌 (Flop, Turn, River)。
/// - 重置新一轮的下注状态。
/// - 确定下一轮第一个行动的玩家 (通常是庄家左边的第一个未弃牌玩家)。
/// - 如果已是 River 结束，则进入 Showdown (摊牌)阶段。
fn advance_to_next_phase(state: &mut GameState) {
    state.cur_bets.iter_mut().for_each(|bet| *bet = 0);
    state.cur_max_bet = 0;
    state.cur_player_idx = None;

    match state.phase {
        GamePhase::PreFlop => {
            state.phase = GamePhase::Flop;
            state.community_cards[0] = Some(state.deck.pop().unwrap());
            state.community_cards[1] = Some(state.deck.pop().unwrap());
            state.community_cards[2] = Some(state.deck.pop().unwrap());
        }
        GamePhase::Flop => {
            state.phase = GamePhase::Turn;
            state.community_cards[3] = Some(state.deck.pop().unwrap());
        }
        GamePhase::Turn => {
            state.phase = GamePhase::River;
            state.community_cards[4] = Some(state.deck.pop().unwrap());
        }
        GamePhase::River => {
            state.phase = GamePhase::Showdown;
            handle_showdown(state);
            return;
        }
        _ => return,
    }

    let mut first_actor_idx = None;
    for i in 0..state.hand_player_order.len() {
        let player_id = state.hand_player_order[i];
        if let Some(player) = state.players.get(&player_id) {
            if matches!(player.state, PlayerState::WaitingForTurn | PlayerState::Acting) {
                first_actor_idx = Some(i);
                break;
            }
        }
    }

    state.cur_player_idx = first_actor_idx;
    if let Some(idx) = first_actor_idx {
        let player_id = state.hand_player_order[idx];
        state.players.get_mut(&player_id).unwrap().state = PlayerState::Acting;
    } else {
        while let Some(pos) = state.community_cards.iter().position(|c| c.is_none()) {
            state.community_cards[pos] = Some(state.deck.pop().unwrap());
        }
        state.phase = GamePhase::Showdown;
        handle_showdown(state);
    }
}

/// 处理摊牌逻辑
///
/// - 找出所有未弃牌的玩家。
/// - 为每个玩家评估他们能组成的最大手牌。
/// - 比较牌力，找到一个或多个赢家。
/// - 分配奖池。
fn handle_showdown(state: &mut GameState) {
    let mut best_rank: Option<HandRank> = None;
    let mut winners = Vec::new();
    let mut player_hands: HashMap<PlayerId, HandRank> = HashMap::new();
    let revealed_community_cards: Vec<Card> = state.community_cards.iter().flatten().cloned().collect();

    for (idx, player_id) in state.hand_player_order.iter().enumerate() {
        let player_state = &state.players.get(player_id).unwrap().state;
        if matches!(player_state, PlayerState::WaitingForTurn | PlayerState::Acting | PlayerState::AllIn) {
            if let (Some(card1), Some(card2)) = state.player_cards[idx] {
                let mut all_cards = revealed_community_cards.clone();
                all_cards.push(card1);
                all_cards.push(card2);
                player_hands.insert(*player_id, find_best_hand(&all_cards));
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

    distribute_pot(state, winners);
    state.phase = GamePhase::HandOver;
}

/// 将奖池分配给赢家
fn distribute_pot(state: &mut GameState, winners: Vec<PlayerId>) {
    if winners.is_empty() { return; }

    let win_amount_per_player = state.pot / winners.len() as u32;
    let remainder = state.pot % winners.len() as u32;

    for (i, winner_id) in winners.iter().enumerate() {
        if let Some(player) = state.players.get_mut(winner_id) {
            player.stack += win_amount_per_player + if i == 0 { remainder } else { 0 };
            player.wins += 1;
        }
    }
    state.pot = 0;
}

// --- 单元测试 ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Rank, Suit};
    use crate::state::{Player, RoomId};
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
            room_id: RoomId::new_v4(),
            players,
            seated_players,
            hand_player_order: vec![],
            player_indices: HashMap::new(),
            phase: GamePhase::WaitingForPlayers,
            pot: 0,
            community_cards: vec![],
            deck: vec![],
            player_cards: vec![],
            cur_bets: vec![],
            cur_player_idx: None,
            cur_max_bet: 0,
            small_blind: 10,
            big_blind: 20,
        };

        (state, player_ids)
    }

    #[test]
    fn test_start_new_hand_normal() {
        // 测试正常情况下的开局
        let (mut state, p_ids) = setup_test_game(&[1000, 1000, 1000, 1000]);
        start_new_hand(&mut state);

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
    fn test_dealer_rotation() {
        // 测试庄家轮换
        let (mut state, p_ids) = setup_test_game(&[1000, 1000, 1000]);
        let initial_dealer = state.seated_players[0];

        start_new_hand(&mut state);
        assert_ne!(state.seated_players[0], initial_dealer, "第一局后庄家应该轮换");
        assert_eq!(state.seated_players[2], initial_dealer, "旧庄家应该移动到队尾");

        let second_hand_dealer = state.seated_players[0];
        start_new_hand(&mut state);
        assert_ne!(state.seated_players[0], second_hand_dealer, "第二局后庄家应该再次轮换");
    }

    #[test]
    fn test_player_action_fold_and_win() {
        // 测试玩家弃牌和最终一人获胜
        let (mut state, p_ids) = setup_test_game(&[1000, 1000, 1000]);
        start_new_hand(&mut state); // p0=庄家, p1=SB, p2=BB. 轮到p0行动

        let p0_id = state.hand_player_order[0];
        let p1_id = state.hand_player_order[1];

        // p0行动 (第一个行动者是p0)
        state.cur_player_idx = Some(0); // 手动设置为p0行动
        handle_player_action(&mut state, p0_id, PlayerAction::Fold);
        assert_eq!(state.players.get(&p0_id).unwrap().state, PlayerState::Folded);

        // p1行动
        handle_player_action(&mut state, p1_id, PlayerAction::Fold);
        assert_eq!(state.players.get(&p1_id).unwrap().state, PlayerState::Folded);

        // 现在只剩p2，游戏应该结束
        assert_eq!(state.phase, GamePhase::HandOver);
        assert_eq!(state.players.get(&p_ids[2]).unwrap().stack, 1010); // p2是大盲，拿回20再赢小盲10
    }

    #[test]
    fn test_betting_round_ends_and_advances_to_flop() {
        // 测试一轮下注结束并进入Flop阶段
        let (mut state, p_ids) = setup_test_game(&[1000, 1000, 1000]);
        start_new_hand(&mut state); // p0=D, p1=SB, p2=BB. 轮到p0行动.

        let p0_id = state.hand_player_order[0];
        let p1_id = state.hand_player_order[1];
        let p2_id = state.hand_player_order[2];

        handle_player_action(&mut state, p0_id, PlayerAction::Call); // p0跟20
        handle_player_action(&mut state, p1_id, PlayerAction::Call); // p1补10
        handle_player_action(&mut state, p2_id, PlayerAction::Check); // p2过牌

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
        start_new_hand(&mut state);
        state.phase = GamePhase::Showdown;
        state.pot = 200;

        // 手动设置牌
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

        let p0_id = state.hand_player_order[0];
        let p1_id = state.hand_player_order[1];
        state.players.get_mut(&p0_id).unwrap().state = PlayerState::WaitingForTurn;
        state.players.get_mut(&p1_id).unwrap().state = PlayerState::WaitingForTurn;

        handle_showdown(&mut state);

        assert_eq!(state.phase, GamePhase::HandOver);
        // 初始1000，减去盲注，再加上奖池
        let p0_stack = state.players.get(&p_ids[0]).unwrap().stack;
        let p1_stack = state.players.get(&p_ids[1]).unwrap().stack;

        // p0是庄家和小盲，p1是大盲
        assert_eq!(p0_stack, 1000 - 10 + 200);
        assert_eq!(p1_stack, 1000 - 20);
    }
}

