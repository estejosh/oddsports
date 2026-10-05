//! Stage 3 — odds score. Pure math. Turns the market consensus into OUR
//! fair-odds estimate plus a confidence rating and the edge vs the best
//! available price. v1: fair odds == no-vig consensus. Later stages
//! (`real_odds_score`) may nudge it with sourced factors.

use crate::aggregate::{moneyline_consensus, prob_to_american};
use oddsports_shared::Game;

/// Below this many books the confidence rating is capped.
pub const FULL_CONFIDENCE_BOOKS: usize = 5;
const LOW_BOOK_CAP: u8 = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct OddsScore {
    pub game_id: String,
    pub home: String,
    pub away: String,
    pub home_prob: f64,
    pub away_prob: f64,
    pub draw_prob: Option<f64>,
    pub books: usize,
    pub dispersion_pp: f64,
    /// 1..=5 — how much to trust the consensus (agreement + sample size).
    pub confidence: u8,
    /// fair prob - best available implied prob, in percentage points.
    /// Positive = the best price on offer is longer than our fair price.
    pub home_edge_pp: f64,
    pub away_edge_pp: f64,
}

/// Confidence from book agreement and sample size. Tight agreement = high
/// confidence; few books caps it.
pub fn confidence_from(dispersion_pp: f64, books: usize) -> u8 {
    let by_dispersion = match dispersion_pp {
        d if d < 0.5 => 5,
        d if d < 1.0 => 4,
        d if d < 2.0 => 3,
        d if d < 3.5 => 2,
        _ => 1,
    };
    if books < FULL_CONFIDENCE_BOOKS {
        by_dispersion.min(LOW_BOOK_CAP)
    } else {
        by_dispersion
    }
}

pub fn score_game(game: &Game) -> Option<OddsScore> {
    let c = moneyline_consensus(game)?;
    Some(OddsScore {
        game_id: game.id.clone(),
        home: game.home.clone(),
        away: game.away.clone(),
        home_prob: c.home,
        away_prob: c.away,
        draw_prob: c.draw,
        books: c.books,
        dispersion_pp: c.dispersion_pp,
        confidence: confidence_from(c.dispersion_pp, c.books),
        home_edge_pp: (c.home - c.best_implied_home) * 100.0,
        away_edge_pp: (c.away - c.best_implied_away) * 100.0,
    })
}

impl OddsScore {
    /// Fair American odds for (home, away).
    pub fn fair_american(&self) -> Option<(i32, i32)> {
        Some((prob_to_american(self.home_prob)?, prob_to_american(self.away_prob)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::testutil::*;

    #[test]
    fn confidence_tracks_dispersion() {
        assert_eq!(confidence_from(0.2, 8), 5);
        assert_eq!(confidence_from(0.7, 8), 4);
        assert_eq!(confidence_from(1.5, 8), 3);
        assert_eq!(confidence_from(3.0, 8), 2);
        assert_eq!(confidence_from(6.0, 8), 1);
    }

    #[test]
    fn few_books_cap_confidence() {
        assert_eq!(confidence_from(0.1, 3), 3);
        assert_eq!(confidence_from(0.1, 4), 3);
        assert_eq!(confidence_from(0.1, 5), 5);
        // cap never raises a low score
        assert_eq!(confidence_from(6.0, 3), 1);
    }

    #[test]
    fn score_reports_fair_odds_and_confidence() {
        let g = two_way(&[
            ("a", -150, 130),
            ("b", -152, 132),
            ("c", -148, 128),
            ("d", -150, 130),
            ("e", -151, 131),
        ]);
        let s = score_game(&g).unwrap();
        assert_eq!(s.books, 5);
        assert!(s.confidence >= 4, "tight market, 5 books: {}", s.confidence);
        let (h, a) = s.fair_american().unwrap();
        assert!(h < 0 && a > 0, "home favoured: {h}/{a}");
        assert!((s.home_prob + s.away_prob - 1.0).abs() < 1e-9);
    }

    #[test]
    fn edge_is_positive_when_best_price_is_longer_than_fair() {
        // three books near -150/+130, one outlier offering home at +110
        let g = two_way(&[("a", -150, 130), ("b", -150, 130), ("c", -150, 130), ("d", 110, -130)]);
        let s = score_game(&g).unwrap();
        assert!(s.home_edge_pp > 0.0, "edge {}", s.home_edge_pp);
    }

    #[test]
    fn no_consensus_means_no_score() {
        let g = two_way(&[("a", -110, -110)]);
        assert!(score_game(&g).is_none());
    }
}
