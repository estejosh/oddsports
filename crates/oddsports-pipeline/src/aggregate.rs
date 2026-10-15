//! Stage 2 — aggregate. Pure math, no I/O, no AI.
//!
//! Turns many books' prices into ONE market consensus per game:
//!   1. American odds -> implied probability
//!   2. per book, remove the vig (normalise all outcomes to sum to 1)
//!   3. average the no-vig probabilities across books
//!   4. dispersion = how much the books disagree (std dev of home no-vig prob)
//!
//! Nothing here names a book. Downstream stages and the published output only
//! ever see the consensus and anonymous aggregates.

use oddsports_shared::Game;
use std::collections::BTreeMap;

/// Need at least this many books before a consensus means anything.
pub const MIN_BOOKS: usize = 3;

/// American odds -> implied probability (vig included). `None` for invalid odds.
pub fn american_to_prob(odds: i32) -> Option<f64> {
    if odds > -100 && odds < 100 {
        return None; // 0 and anything inside (-100, 100) is not a real American price
    }
    let o = odds as f64;
    Some(if odds < 0 { -o / (-o + 100.0) } else { 100.0 / (o + 100.0) })
}

/// Probability -> American odds, rounded to the nearest integer. `None` outside (0, 1).
pub fn prob_to_american(p: f64) -> Option<i32> {
    if !(p > 0.0 && p < 1.0) {
        return None;
    }
    let v = if p >= 0.5 { -(p / (1.0 - p)) * 100.0 } else { ((1.0 - p) / p) * 100.0 };
    Some(v.round() as i32)
}

/// Scale probabilities so they sum to 1 (this is what removes the vig).
pub fn normalize(probs: &[f64]) -> Vec<f64> {
    let sum: f64 = probs.iter().sum();
    if sum <= 0.0 {
        return probs.to_vec();
    }
    probs.iter().map(|p| p / sum).collect()
}

/// Population standard deviation.
pub fn std_dev(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mean = xs.iter().sum::<f64>() / xs.len() as f64;
    (xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / xs.len() as f64).sqrt()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Consensus {
    pub home: f64,
    pub away: f64,
    /// Present for three-way markets (soccer).
    pub draw: Option<f64>,
    pub books: usize,
    /// Std dev of the books' no-vig home probability, in percentage points.
    pub dispersion_pp: f64,
    /// Lowest vig-included implied probability on offer for each side — i.e. the
    /// best price a bettor could take. Anonymous: no book attached.
    pub best_implied_home: f64,
    pub best_implied_away: f64,
}

struct BookView {
    home_nv: f64,
    away_nv: f64,
    draw_nv: Option<f64>,
    home_raw: f64,
    away_raw: f64,
    outcomes: usize,
}

/// No-vig moneyline consensus for a game, or `None` if fewer than `MIN_BOOKS`
/// books quote a complete, comparable market.
pub fn moneyline_consensus(game: &Game) -> Option<Consensus> {
    // book -> (outcome name, implied prob)
    let mut by_book: BTreeMap<&str, Vec<(String, f64)>> = BTreeMap::new();
    for l in &game.moneyline_lines {
        let (Some(name), Some(p)) = (l.outcome.as_deref(), american_to_prob(l.american_odds)) else {
            continue;
        };
        by_book.entry(l.book.as_str()).or_default().push((name.to_lowercase(), p));
    }

    let home_key = game.home.to_lowercase();
    let away_key = game.away.to_lowercase();
    let mut views: Vec<BookView> = Vec::new();

    for outcomes in by_book.values() {
        let mut names: Vec<&str> = outcomes.iter().map(|(n, _)| n.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        if names.len() != outcomes.len() {
            continue; // duplicate outcome from one book: ambiguous, skip
        }
        let find = |k: &str| outcomes.iter().find(|(n, _)| n == k).map(|(_, p)| *p);
        let (Some(h_raw), Some(a_raw)) = (find(&home_key), find(&away_key)) else {
            continue;
        };
        let probs: Vec<f64> = outcomes.iter().map(|(_, p)| *p).collect();
        let nv = normalize(&probs);
        let nv_of = |k: &str| outcomes.iter().position(|(n, _)| n == k).map(|i| nv[i]);
        let draw_nv = if outcomes.len() == 3 {
            outcomes
                .iter()
                .position(|(n, _)| *n != home_key && *n != away_key)
                .map(|i| nv[i])
        } else {
            None
        };
        let (Some(home_nv), Some(away_nv)) = (nv_of(&home_key), nv_of(&away_key)) else {
            continue;
        };
        views.push(BookView {
            home_nv,
            away_nv,
            draw_nv,
            home_raw: h_raw,
            away_raw: a_raw,
            outcomes: outcomes.len(),
        });
    }

    // Keep only books quoting the modal market shape (2-way vs 3-way) so the
    // averaged probabilities still sum to 1.
    let n2 = views.iter().filter(|v| v.outcomes == 2).count();
    let n3 = views.iter().filter(|v| v.outcomes == 3).count();
    let want = if n3 >= n2 { 3 } else { 2 };
    views.retain(|v| v.outcomes == want);
    if views.len() < MIN_BOOKS {
        return None;
    }

    let n = views.len() as f64;
    let home = views.iter().map(|v| v.home_nv).sum::<f64>() / n;
    let away = views.iter().map(|v| v.away_nv).sum::<f64>() / n;
    let draw = if want == 3 {
        Some(views.iter().filter_map(|v| v.draw_nv).sum::<f64>() / n)
    } else {
        None
    };
    let home_series: Vec<f64> = views.iter().map(|v| v.home_nv).collect();
    let best_home = views.iter().map(|v| v.home_raw).fold(f64::INFINITY, f64::min);
    let best_away = views.iter().map(|v| v.away_raw).fold(f64::INFINITY, f64::min);

    Some(Consensus {
        home,
        away,
        draw,
        books: views.len(),
        dispersion_pp: std_dev(&home_series) * 100.0,
        best_implied_home: best_home,
        best_implied_away: best_away,
    })
}

#[cfg(test)]
pub(crate) mod testutil {
    use oddsports_shared::{BookLine, Game, Sport};

    pub fn line(book: &str, outcome: &str, odds: i32) -> BookLine {
        BookLine {
            book: book.into(),
            american_odds: odds,
            line: None,
            outcome: Some(outcome.into()),
            fetched_at: "2026-10-01T12:00:00Z".into(),
        }
    }

    pub fn game_with(moneyline: Vec<BookLine>) -> Game {
        Game {
            id: "g1".into(),
            sport: Sport::Nba,
            home: "Celtics".into(),
            away: "Knicks".into(),
            starts_at: "2026-10-02T23:30:00Z".into(),
            spread_lines: vec![],
            moneyline_lines: moneyline,
            total_lines: vec![],
        }
    }

    /// Two-way market for N books at the given (home, away) prices.
    pub fn two_way(books: &[(&str, i32, i32)]) -> Game {
        let mut v = Vec::new();
        for (b, h, a) in books {
            v.push(line(b, "Celtics", *h));
            v.push(line(b, "Knicks", *a));
        }
        game_with(v)
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn american_to_prob_known_values() {
        assert!(close(american_to_prob(-110).unwrap(), 110.0 / 210.0));
        assert!(close(american_to_prob(100).unwrap(), 0.5));
        assert!(close(american_to_prob(-200).unwrap(), 2.0 / 3.0));
        assert!(close(american_to_prob(300).unwrap(), 0.25));
    }

    #[test]
    fn invalid_american_odds_rejected() {
        assert!(american_to_prob(0).is_none());
        assert!(american_to_prob(50).is_none());
        assert!(american_to_prob(-99).is_none());
    }

    #[test]
    fn prob_to_american_round_trips() {
        assert_eq!(prob_to_american(0.5), Some(-100));
        assert_eq!(prob_to_american(2.0 / 3.0), Some(-200));
        assert_eq!(prob_to_american(0.25), Some(300));
        assert!(prob_to_american(0.0).is_none());
        assert!(prob_to_american(1.0).is_none());
    }

    #[test]
    fn standard_minus_110_both_sides_has_no_edge_after_vig_removed() {
        // -110/-110 carries ~4.76% vig; no-vig fair is exactly 50/50.
        let raw = american_to_prob(-110).unwrap();
        assert!(raw * 2.0 > 1.04);
        let nv = normalize(&[raw, raw]);
        assert!(close(nv[0], 0.5) && close(nv[1], 0.5));
    }

    #[test]
    fn no_vig_probabilities_sum_to_one() {
        let nv = normalize(&[american_to_prob(-150).unwrap(), american_to_prob(130).unwrap()]);
        assert!(close(nv.iter().sum::<f64>(), 1.0));
        assert!(nv[0] > nv[1]);
    }

    #[test]
    fn consensus_averages_no_vig_across_books() {
        let g = two_way(&[("a", -110, -110), ("b", -120, 100), ("c", -105, -115)]);
        let c = moneyline_consensus(&g).unwrap();
        assert_eq!(c.books, 3);
        assert!(close(c.home + c.away, 1.0));
        // book a is exactly 0.5 no-vig; b and c lean home, so consensus home > 0.5
        assert!(c.home > 0.5);
        assert!(c.draw.is_none());
    }

    #[test]
    fn identical_books_have_zero_dispersion() {
        let g = two_way(&[("a", -110, -110), ("b", -110, -110), ("c", -110, -110)]);
        let c = moneyline_consensus(&g).unwrap();
        assert!(c.dispersion_pp.abs() < 1e-9);
        assert!(close(c.home, 0.5));
    }

    #[test]
    fn disagreeing_books_have_positive_dispersion() {
        let tight = two_way(&[("a", -150, 130), ("b", -152, 132), ("c", -148, 128)]);
        let wide = two_way(&[("a", -250, 210), ("b", -120, 100), ("c", 110, -130)]);
        let t = moneyline_consensus(&tight).unwrap().dispersion_pp;
        let w = moneyline_consensus(&wide).unwrap().dispersion_pp;
        assert!(w > t * 5.0, "wide {w} should dwarf tight {t}");
    }

    #[test]
    fn too_few_books_gives_no_consensus() {
        let g = two_way(&[("a", -110, -110), ("b", -110, -110)]);
        assert!(moneyline_consensus(&g).is_none());
    }

    #[test]
    fn book_missing_a_side_is_skipped() {
        let mut g = two_way(&[("a", -110, -110), ("b", -110, -110), ("c", -110, -110)]);
        g.moneyline_lines.push(line("d", "Celtics", -300)); // no Knicks quote from d
        let c = moneyline_consensus(&g).unwrap();
        assert_eq!(c.books, 3);
    }

    #[test]
    fn three_way_market_includes_draw_and_sums_to_one() {
        let mut v = Vec::new();
        for b in ["a", "b", "c"] {
            v.push(line(b, "Celtics", 150));
            v.push(line(b, "Knicks", 200));
            v.push(line(b, "Draw", 230));
        }
        let c = moneyline_consensus(&game_with(v)).unwrap();
        assert!(c.draw.is_some());
        assert!(close(c.home + c.away + c.draw.unwrap(), 1.0));
    }

    #[test]
    fn best_price_is_lowest_implied_and_carries_no_book_name() {
        let g = two_way(&[("a", -150, 130), ("b", -140, 120), ("c", -160, 140)]);
        let c = moneyline_consensus(&g).unwrap();
        assert!(close(c.best_implied_home, american_to_prob(-140).unwrap()));
        assert!(close(c.best_implied_away, american_to_prob(140).unwrap()));
    }

    #[test]
    fn std_dev_basic() {
        assert!(close(std_dev(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]), 2.0));
        assert_eq!(std_dev(&[]), 0.0);
    }
}
