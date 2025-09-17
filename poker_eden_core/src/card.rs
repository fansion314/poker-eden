use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
// --- 核心数据结构定义 ---

/// 花色 (Suit)
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Serialize, Deserialize)]
pub enum Suit {
    Spade,   // 黑桃 ♠
    Heart,   // 红心 ♥
    Club,    // 梅花 ♣
    Diamond, // 方块 ♦
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
    rank: Rank,
    suit: Suit,
}

/// 牌型等级 (HandRank)
/// 这个枚举的设计是核心所在。
/// 1. 变体的顺序从大到小排列，可以直接利用 `Ord` 进行比较。
/// 2. 变体内部存储了比较所需的所有信息（例如对子的大小、三条的大小、踢脚牌等）。
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
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
        let symbol = match self {
            Suit::Spade => "♠",
            Suit::Heart => "♥",
            Suit::Club => "♣",
            Suit::Diamond => "♦",
        };
        write!(f, "{}", symbol)
    }
}

impl fmt::Display for Rank {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let representation = match self {
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
        };
        write!(f, "{}", representation)
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{}", self.rank, self.suit)
    }
}

// --- 牌型评估逻辑 ---

/// 评估一手 5 张牌的牌型
///
/// # Panics
/// 如果传入的牌不是 5 张，则会 panic。
pub fn evaluate_hand(hand: &[Card]) -> HandRank {
    assert_eq!(hand.len(), 5, "评估的牌必须是5张");

    let mut cards = hand.to_vec();
    // 从大到小排序，方便处理
    cards.sort_by(|a, b| b.rank.cmp(&a.rank));
    let ranks: Vec<Rank> = cards.iter().map(|c| c.rank).collect();

    // 1. 检查同花和同花顺
    let is_flush = cards.windows(2).all(|w| w[0].suit == w[1].suit);

    // 2. 检查顺子
    let is_straight = ranks.windows(2).all(|w| w[0] as i32 == w[1] as i32 + 1)
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

#[cfg(test)]
mod tests {
    use crate::card::Rank::*;
    use crate::card::Suit::*;
    use crate::card::*;

    #[test]
    fn main() {
        // 皇家同花顺
        let royal_flush = vec![
            Card { rank: Ace, suit: Spade },
            Card { rank: King, suit: Spade },
            Card { rank: Queen, suit: Spade },
            Card { rank: Jack, suit: Spade },
            Card { rank: Ten, suit: Spade },
        ];

        // 葫芦
        let full_house = vec![
            Card { rank: Jack, suit: Heart },
            Card { rank: Jack, suit: Diamond },
            Card { rank: Jack, suit: Club },
            Card { rank: Two, suit: Spade },
            Card { rank: Two, suit: Heart },
        ];

        // 两对
        let two_pair = vec![
            Card { rank: King, suit: Heart },
            Card { rank: King, suit: Diamond },
            Card { rank: Five, suit: Club },
            Card { rank: Five, suit: Spade },
            Card { rank: Four, suit: Heart },
        ];

        let rank1 = evaluate_hand(&royal_flush);
        let rank2 = evaluate_hand(&full_house);
        let rank3 = evaluate_hand(&two_pair);

        println!("手牌 1: [{}]", royal_flush.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" "));
        println!("牌型: {:?}", rank1);
        println!();

        println!("手牌 2: [{}]", full_house.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" "));
        println!("牌型: {:?}", rank2);
        println!();

        println!("手牌 3: [{}]", two_pair.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" "));
        println!("牌型: {:?}", rank3);
        println!();


        // 比较牌型大小
        println!("皇家同花顺 > 葫芦? {}", rank1 > rank2);
        assert!(rank1 > rank2);
        println!("葫芦 > 两对? {}", rank2 > rank3);
        assert!(rank2 > rank3);
        println!("皇家同花顺 > 两对? {}", rank1 > rank3);
        assert!(rank1 > rank3);

        // 比较相同牌型但不同大小的情况
        let pair_of_kings = evaluate_hand(&[
            Card { rank: King, suit: Spade }, Card { rank: King, suit: Heart },
            Card { rank: Ten, suit: Spade }, Card { rank: Four, suit: Spade }, Card { rank: Two, suit: Spade },
        ]);
        let pair_of_queens = evaluate_hand(&[
            Card { rank: Queen, suit: Spade }, Card { rank: Queen, suit: Heart },
            Card { rank: Ten, suit: Spade }, Card { rank: Four, suit: Spade }, Card { rank: Two, suit: Spade },
        ]);

        println!("\nK对 vs Q对: K对 > Q对? {}", pair_of_kings > pair_of_queens);
        assert!(pair_of_kings > pair_of_queens);
    }
}
