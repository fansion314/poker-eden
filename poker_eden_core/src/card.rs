use rand::prelude::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
// --- 核心数据结构定义 ---

/// 花色 (Suit)
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Serialize, Deserialize)]
pub enum Suit {
    Spade,   // 黑桃 ♠️
    Heart,   // 红心 ♥️
    Club,    // 梅花 ♣️
    Diamond, // 方块 ♦️
}

/// 点数 (Rank)
/// Ace 可以是最大也可以是最小 (在 A-2-3-4-5 顺子中)
/// Ord 的派生让 Ace 默认是最大的
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Serialize, Deserialize)]
pub enum Rank {
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Jack,
    Queen,
    King,
    Ace,
}

/// 单张扑克牌 (Card)
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Serialize, Deserialize)]
pub struct Card {
    pub rank: Rank,
    pub suit: Suit,
}

impl Card {
    pub fn new(rank: Rank, suit: Suit) -> Card {
        Card { rank, suit }
    }
}

/// 牌型等级 (HandRank)
/// 这个枚举的设计是核心所在。
/// 1. 变体的顺序从大到小排列，可以直接利用 `Ord` 进行比较。
/// 2. 变体内部存储了比较所需的所有信息（例如对子的大小、三条的大小、踢脚牌等）。
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Serialize, Deserialize)]
pub enum HandRank {
    HighCard(Rank, Rank, Rank, Rank, Rank),          // 高牌
    OnePair(Rank, Rank, Rank, Rank),                 // 一对
    TwoPair(Rank, Rank, Rank),                       // 两对
    ThreeOfAKind(Rank, Rank, Rank),                  // 三条
    Straight(Rank),                                  // 顺子 (最高牌的点数)
    Flush(Rank, Rank, Rank, Rank, Rank),             // 同花
    FullHouse(Rank, Rank),                           // 葫芦 (三条的点数, 对子的点数)
    FourOfAKind(Rank, Rank),                         // 四条 (四条的点数, 踢脚牌)
    StraightFlush(Rank),                             // 同花顺 (最高牌的点数)
    RoyalFlush,                                      // 皇家同花顺
}

// --- 实现辅助功能 ---

impl fmt::Display for Suit {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", match self {
            Suit::Spade => "♠️",
            Suit::Heart => "♥️",
            Suit::Club => "♣️",
            Suit::Diamond => "♦️",
        })
    }
}

impl fmt::Display for Rank {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", match self {
            Rank::Two => "2",
            Rank::Three => "3",
            Rank::Four => "4",
            Rank::Five => "5",
            Rank::Six => "6",
            Rank::Seven => "7",
            Rank::Eight => "8",
            Rank::Nine => "9",
            Rank::Ten => "T",
            Rank::Jack => "J",
            Rank::Queen => "Q",
            Rank::King => "K",
            Rank::Ace => "A",
        })
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{}", self.suit, self.rank)
    }
}

impl fmt::Display for HandRank {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", match self {
            HandRank::HighCard(..) => "高牌".to_string(),
            HandRank::OnePair(r1, ..) => format!("一对({})", r1),
            HandRank::TwoPair(r1, r2, ..) => format!("两对({},{})", r1, r2),
            HandRank::ThreeOfAKind(r1, ..) => format!("三条({})", r1),
            HandRank::Straight(..) => "顺子".to_string(),
            HandRank::Flush(..) => "同花".to_string(),
            HandRank::FullHouse(..) => "葫芦".to_string(),
            HandRank::FourOfAKind(..) => "四条".to_string(),
            HandRank::StraightFlush(..) => "同花顺".to_string(),
            HandRank::RoyalFlush => "皇家同花顺".to_string(),
        })
    }
}

// --- 随机牌组生成 ---

/// 创建一副完整的 52 张扑克牌
fn create_deck() -> Vec<Card> {
    let suits = [Suit::Spade, Suit::Heart, Suit::Club, Suit::Diamond];
    let ranks = [
        Rank::Two, Rank::Three, Rank::Four, Rank::Five, Rank::Six, Rank::Seven,
        Rank::Eight, Rank::Nine, Rank::Ten, Rank::Jack, Rank::Queen, Rank::King, Rank::Ace,
    ];
    let mut deck = Vec::with_capacity(52);
    for &suit in &suits {
        for &rank in &ranks {
            deck.push(Card { rank, suit });
        }
    }
    deck
}

/// 从一副新牌中随机生成并返回 2*k+5 张牌
pub fn generate_random_hand(k_players: usize) -> Vec<Card> {
    // 德州扑克通常支持 2 到 10 名玩家
    assert!(k_players >= 2 && k_players <= 10, "Number of players must be between 2 and 10.");

    let mut deck = create_deck();
    let mut rng = rand::rng();
    deck.shuffle(&mut rng);

    let total_cards = 2 * k_players + 5;
    let mut cards = vec![Card { rank: Rank::Ace, suit: Suit::Heart }; total_cards];

    for i in 0..2 {
        for j in 0..k_players {
            if let Some(card) = deck.pop() {
                cards[j * 2 + i] = card;
            }
        }
    }

    // 发公共牌 (Community Cards)
    deck.pop(); // 烧掉一张牌 (Flop burn)
    for i in (2 * k_players)..(2 * k_players + 3) {
        if let Some(card) = deck.pop() {
            cards[i] = card;
        }
    }
    deck.pop(); // 再烧掉一张牌 (Turn burn)
    if let Some(card) = deck.pop() {
        cards[2 * k_players + 3] = card;
    }
    deck.pop(); // 最后烧掉一张牌 (River burn)
    if let Some(card) = deck.pop() {
        cards[2 * k_players + 4] = card;
    }

    cards
}

// --- 牌型评估逻辑 ---

/// 从 5 到 7 张牌中找出最优的 5 张牌组合牌力
/// 这是德州扑克规则的核心评估函数
///
/// # Panics
/// 如果牌数少于 5 或多于 7，则会 panic。
pub fn find_best_hand(all_cards: &[Card]) -> HandRank {
    let card_count = all_cards.len();
    assert!(card_count >= 5 && card_count <= 7, "牌数必须在5到7张之间");

    if card_count == 5 {
        return evaluate_5_card_hand(all_cards);
    }

    // 通过生成所有5张牌的组合来找到最佳手牌。
    // 这是唯一确保正确性的方法，因为贪心算法（如移除最小的牌）可能会破坏顺子或同花。
    let combinations = get_combinations(all_cards, 5);

    combinations.into_iter()
        .map(|hand| evaluate_5_card_hand(&hand))
        .max() // HandRank 派生了 Ord，可以直接找到最大的
        .unwrap() // 因为我们知道至少会有一个组合，所以 unwrap 是安全的
}

/// 评估一手 5 张牌的牌型 (原 evaluate_hand 函数)
fn evaluate_5_card_hand(hand: &[Card]) -> HandRank {
    assert_eq!(hand.len(), 5, "评估的牌必须是5张");

    let mut cards = hand.to_vec();
    // 从大到小排序，方便处理
    cards.sort_by(|a, b| b.rank.cmp(&a.rank));
    let ranks: Vec<Rank> = cards.iter().map(|c| c.rank).collect();

    // 1. 检查同花和同花顺
    let is_flush = cards.windows(2).all(|w| w[0].suit == w[1].suit);

    // 2. 检查顺子
    let is_straight = ranks.windows(2).all(|w| w[0] as u8 == w[1] as u8 + 1)
        // 特殊情况: A-2-3-4-5
        || ranks == [Rank::Ace, Rank::Five, Rank::Four, Rank::Three, Rank::Two];

    let high_card = if ranks == [Rank::Ace, Rank::Five, Rank::Four, Rank::Three, Rank::Two] {
        Rank::Five // A-5 顺子中，5是最大的牌
    } else {
        ranks[0]
    };

    if is_straight && is_flush {
        return if high_card == Rank::Ace {
            HandRank::RoyalFlush
        } else {
            HandRank::StraightFlush(high_card)
        };
    }

    // 3. 统计点数出现次数，用于判断四条、葫芦、三条、两对、一对
    let mut counts: HashMap<Rank, u8> = HashMap::new();
    for rank in &ranks {
        *counts.entry(*rank).or_insert(0) += 1;
    }

    // 将统计结果转换为 (出现次数, 点数) 的元组列表，并按次数和点数排序
    let mut sorted_counts: Vec<(u8, Rank)> = counts.into_iter().map(|(r, c)| (c, r)).collect();
    sorted_counts.sort_by(|a, b| b.cmp(a)); // 先按次数，再按点数从大到小排

    match sorted_counts[0].0 {
        4 => { // 四条
            HandRank::FourOfAKind(sorted_counts[0].1, sorted_counts[1].1)
        }
        3 => { // 葫芦或三条
            if sorted_counts[1].0 == 2 {
                HandRank::FullHouse(sorted_counts[0].1, sorted_counts[1].1)
            } else {
                HandRank::ThreeOfAKind(sorted_counts[0].1, sorted_counts[1].1, sorted_counts[2].1)
            }
        }
        2 => { // 两对或一对
            if sorted_counts[1].0 == 2 {
                HandRank::TwoPair(sorted_counts[0].1, sorted_counts[1].1, sorted_counts[2].1)
            } else {
                HandRank::OnePair(
                    sorted_counts[0].1,
                    sorted_counts[1].1,
                    sorted_counts[2].1,
                    sorted_counts[3].1,
                )
            }
        }
        _ => { // 剩下的情况
            if is_flush {
                HandRank::Flush(ranks[0], ranks[1], ranks[2], ranks[3], ranks[4])
            } else if is_straight {
                HandRank::Straight(high_card)
            } else {
                HandRank::HighCard(ranks[0], ranks[1], ranks[2], ranks[3], ranks[4])
            }
        }
    }
}

/// 辅助函数：从一个切片中生成所有大小为 k 的组合
fn get_combinations<T: Clone>(data: &[T], k: usize) -> Vec<Vec<T>> {
    if k == 0 {
        return vec![vec![]];
    }
    if data.len() < k {
        return vec![];
    }

    let mut result = vec![];
    let (first, rest) = data.split_at(1);

    // 包含第一个元素的组合
    let mut combinations_with_first = get_combinations(rest, k - 1);
    for combo in &mut combinations_with_first {
        combo.insert(0, first[0].clone());
    }
    result.append(&mut combinations_with_first);

    // 不包含第一个元素的组合
    if data.len() > k {
        let mut combinations_without_first = get_combinations(rest, k);
        result.append(&mut combinations_without_first);
    }

    result
}

// --- 单元测试 ---

#[cfg(test)]
mod tests {
    use super::*;
    // 导入父模块的所有内容
    use Rank::*;
    use Suit::*;

    // 辅助函数，用于快速创建牌
    fn card(rank: Rank, suit: Suit) -> Card {
        Card { rank, suit }
    }

    // --- 5张牌评估测试 ---
    #[test]
    fn test_royal_flush() {
        let hand = [card(Ten, Spade), card(Ace, Spade), card(Queen, Spade), card(King, Spade), card(Jack, Spade)];
        assert_eq!(evaluate_5_card_hand(&hand), HandRank::RoyalFlush);
    }

    #[test]
    fn test_straight_flush() {
        let hand = [card(Nine, Heart), card(Ten, Heart), card(Eight, Heart), card(Jack, Heart), card(Seven, Heart)];
        assert_eq!(evaluate_5_card_hand(&hand), HandRank::StraightFlush(Jack));
    }

    #[test]
    fn test_ace_low_straight_flush() {
        let hand = [card(Ace, Club), card(Two, Club), card(Three, Club), card(Four, Club), card(Five, Club)];
        assert_eq!(evaluate_5_card_hand(&hand), HandRank::StraightFlush(Five));
    }

    #[test]
    fn test_four_of_a_kind() {
        let hand = [card(Ace, Spade), card(Ace, Heart), card(Ace, Diamond), card(Ace, Club), card(King, Spade)];
        assert_eq!(evaluate_5_card_hand(&hand), HandRank::FourOfAKind(Ace, King));
    }

    #[test]
    fn test_full_house() {
        let hand = [card(King, Spade), card(King, Heart), card(King, Diamond), card(Queen, Club), card(Queen, Spade)];
        assert_eq!(evaluate_5_card_hand(&hand), HandRank::FullHouse(King, Queen));
    }

    #[test]
    fn test_flush() {
        let hand = [card(Two, Diamond), card(Five, Diamond), card(Eight, Diamond), card(Jack, Diamond), card(Ace, Diamond)];
        assert_eq!(evaluate_5_card_hand(&hand), HandRank::Flush(Ace, Jack, Eight, Five, Two));
    }

    #[test]
    fn test_straight() {
        let hand = [card(Ten, Spade), card(Nine, Heart), card(Eight, Diamond), card(Seven, Club), card(Six, Spade)];
        assert_eq!(evaluate_5_card_hand(&hand), HandRank::Straight(Ten));
    }

    #[test]
    fn test_ace_low_straight() {
        let hand = [card(Ace, Spade), card(Two, Heart), card(Three, Diamond), card(Four, Club), card(Five, Spade)];
        assert_eq!(evaluate_5_card_hand(&hand), HandRank::Straight(Five));
    }

    #[test]
    fn test_three_of_a_kind() {
        let hand = [card(Ten, Spade), card(Ten, Heart), card(Ten, Diamond), card(Jack, Club), card(Two, Spade)];
        assert_eq!(evaluate_5_card_hand(&hand), HandRank::ThreeOfAKind(Ten, Jack, Two));
    }

    #[test]
    fn test_two_pair() {
        let hand = [card(Jack, Spade), card(Jack, Heart), card(Nine, Diamond), card(Nine, Club), card(Ten, Spade)];
        assert_eq!(evaluate_5_card_hand(&hand), HandRank::TwoPair(Jack, Nine, Ten));
    }

    #[test]
    fn test_one_pair() {
        let hand = [card(Ace, Spade), card(Ace, Heart), card(King, Diamond), card(Queen, Club), card(Jack, Spade)];
        assert_eq!(evaluate_5_card_hand(&hand), HandRank::OnePair(Ace, King, Queen, Jack));
    }

    #[test]
    fn test_high_card() {
        let hand = [card(King, Spade), card(Queen, Heart), card(Jack, Diamond), card(Nine, Club), card(Seven, Spade)];
        assert_eq!(evaluate_5_card_hand(&hand), HandRank::HighCard(King, Queen, Jack, Nine, Seven));
    }

    // --- 7选5评估测试 ---

    #[test]
    fn test_best_hand_from_seven_is_flush() {
        // 7张牌中可以组成同花，但也有对子，需要正确选择同花
        let cards = [
            card(Ace, Heart), card(King, Heart),    // Player's hand
            card(Ten, Heart), card(Two, Heart), card(Five, Heart), // Board
            card(Ace, Spade), card(Ten, Club) // Extra cards on board
        ];
        assert_eq!(find_best_hand(&cards), HandRank::Flush(Ace, King, Ten, Five, Two));
    }

    #[test]
    fn test_best_hand_from_seven_is_full_house() {
        // 7张牌中有三条和两对，可以组成葫芦
        let cards = [
            card(Ten, Spade), card(Ten, Heart),   // Player's hand
            card(Jack, Club), card(Jack, Diamond), card(Ten, Diamond), // Board
            card(Two, Club), card(Three, Spade)   // Board
        ];
        assert_eq!(find_best_hand(&cards), HandRank::FullHouse(Ten, Jack));
    }

    #[test]
    fn test_best_hand_from_seven_plays_the_board() {
        // 玩家手牌很小，最好的牌是桌面上的顺子
        let cards = [
            card(Two, Spade), card(Two, Heart), // Player's hand
            card(Ten, Club), card(Jack, Diamond), card(Queen, Heart), // Board
            card(King, Spade), card(Ace, Club) // Board
        ];
        // 最佳手牌是 A-K-Q-J-T 顺子，玩家的对2没用
        assert_eq!(find_best_hand(&cards), HandRank::Straight(Ace));
    }

    #[test]
    fn test_best_hand_from_six() {
        let cards = [
            card(Ace, Spade), card(Ace, Heart), card(King, Spade),
            card(King, Heart), card(Queen, Spade), card(Jack, Spade)
        ];
        // 最佳牌型是 A-A-K-K-Q (两对)
        assert_eq!(find_best_hand(&cards), HandRank::TwoPair(Ace, King, Queen));
    }

    // --- 牌力比较测试 ---
    #[test]
    fn test_rank_comparison() {
        let full_house_kings = HandRank::FullHouse(King, Two);
        let full_house_queens = HandRank::FullHouse(Queen, Ace);
        let flush_king_high = HandRank::Flush(King, Jack, Ten, Five, Two);
        let flush_queen_high = HandRank::Flush(Queen, Jack, Ten, Five, Two);

        assert!(HandRank::RoyalFlush > HandRank::StraightFlush(King));
        assert!(HandRank::StraightFlush(King) > HandRank::FourOfAKind(Ace, King));
        assert!(full_house_kings > full_house_queens); // K葫芦 > Q葫芦
        assert!(flush_king_high > flush_queen_high); // K同花 > Q同花
    }
}
