//! Deterministic model layer. PRD Section 6 #2: "LLM writes prose, code
//! computes numbers." Everything here is pure math — no AI calls, ever.
//!
//! v1 model is intentionally simple (market-consensus based): fair line =
//! median across books; edge = deviation of best available price from fair.
//! TODO(fable): replace with real projections per sport (pace/weather/injury
//! adjusted) — the `ModelOutput` interface stays the same.

use chrono::Utc;
use oddsports_shared::{
    Factor, FactorDirection, Game, LinePoint, MarketType, ModelOutput, PickTeam,
};
use std::collections::HashMap;

const MAX_UNITS: f64 = 3.0; // hard cap regardless of edge — bankroll discipline
const KELLY_FRACTION: f64 = 0.25; // quarter-Kelly
const MIN_BOOKS: usize = 3; // need consensus to say anything
const MIN_EDGE: f64 = 0.5; // points of line value
const STEAM_THRESHOLD: f64 = 1.0; // points of median movement = steam

/// `history`: median home-line snapshots per game (from store::line_history),
/// oldest first. Empty map = no snapshot data yet; model runs on consensus only.
pub fn run_model(games: &[Game], history: &HashMap<String, Vec<LinePoint>>) -> Vec<ModelOutput> {
    let mut outputs: Vec<ModelOutput> = games
        .iter()
        .filter_map(|g| analyze_spread(g, history.get(g.id.as_str()).map(Vec::as_slice).unwrap_or(&[])))
        .collect();
    // Highest edge first — if the token budget degrades, we drop from the tail.
    outputs.sort_by(|a, b| b.edge_pct.partial_cmp(&a.edge_pct).unwrap_or(std::cmp::Ordering::Equal));
    outputs
}

fn analyze_spread(game: &Game, history: &[LinePoint]) -> Option<ModelOutput> {
    let mut points: Vec<(f64, &str)> = game
        .spread_lines
        .iter()
        .filter_map(|l| l.line.map(|p| (p, l.book.as_str())))
        .collect();
    if points.len() < MIN_BOOKS {
        return None;
    }
    points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let fair_line = median(&points);
    let (market_line, best_book) = points
        .iter()
        .max_by(|a, b| {
            (a.0 - fair_line)
                .abs()
                .partial_cmp(&(b.0 - fair_line).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()?;

    let edge = ((market_line - fair_line).abs() * 100.0).round() / 100.0;
    if edge < MIN_EDGE {
        return None;
    }

    let confidence: u8 = match edge {
        e if e >= 2.0 => 4,
        e if e >= 1.5 => 3,
        e if e >= 1.0 => 2,
        _ => 1,
    };
    let suggested_units = (edge * KELLY_FRACTION * 2.0 * 10.0).round() / 10.0;
    let suggested_units = suggested_units.min(MAX_UNITS);

    let (pick_team, side) = if market_line > fair_line {
        (PickTeam::Home, format!("{} {}", game.home, fmt_line(market_line)))
    } else {
        (PickTeam::Away, format!("{} {}", game.away, fmt_line(-market_line)))
    };

    let mut factors = vec![Factor {
        name: "book consensus deviation".into(),
        direction: FactorDirection::For,
        weight: 1.0,
        detail: format!(
            "{best_book} posts {} vs market median {} across {} books",
            fmt_line(market_line),
            fmt_line(fair_line),
            points.len()
        ),
    }];

    // Signal 2 — steam: sustained median movement across snapshots means sharp
    // money is pushing the number. Confirms the pick if the market is moving
    // TOWARD our side (making our stale price better); contradicts if moving away.
    let mut confidence = confidence;
    if let (Some(first), Some(last)) = (history.first(), history.last()) {
        let movement = last.line - first.line; // home-perspective points
        if movement.abs() >= STEAM_THRESHOLD {
            // Home line dropping (more negative) = money on home; rising = money on away.
            let steam_on_home = movement < 0.0;
            let agrees = steam_on_home == (pick_team == PickTeam::Home);
            factors.push(Factor {
                name: "line steam".into(),
                direction: if agrees { FactorDirection::For } else { FactorDirection::Against },
                weight: (movement.abs() / 2.0).min(1.0),
                detail: format!(
                    "market median moved {} → {} over {} snapshots ({})",
                    fmt_line(first.line),
                    fmt_line(last.line),
                    history.len(),
                    if agrees { "toward our side" } else { "against our side" }
                ),
            });
            confidence = if agrees { (confidence + 1).min(5) } else { confidence.saturating_sub(1).max(1) };
        }
    }

    let line_history = if history.is_empty() {
        vec![LinePoint { at: Utc::now().to_rfc3339(), line: market_line }]
    } else {
        history.to_vec()
    };

    Some(ModelOutput {
        game_id: game.id.clone(),
        market: MarketType::Spread,
        side,
        pick_team,
        picked_line: market_line,
        fair_line,
        edge_pct: edge,
        confidence,
        suggested_units,
        factors,
        line_history,
    })
}

/// Bankroll-scaled sizing for Sharp tier. Pure arithmetic (PRD 6 — no AI).
pub fn scale_units_to_bankroll(units: f64, bankroll_usd: f64) -> f64 {
    let unit_size = bankroll_usd / 100.0; // 1 unit = 1% of bankroll
    (units * unit_size * 100.0).round() / 100.0
}

fn median(sorted: &[(f64, &str)]) -> f64 {
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid].0
    } else {
        (sorted[mid - 1].0 + sorted[mid].0) / 2.0
    }
}

fn fmt_line(n: f64) -> String {
    if n > 0.0 {
        format!("+{n}")
    } else {
        format!("{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oddsports_shared::{BookLine, Sport};

    fn game_with_lines(id: &str, lines: Vec<f64>) -> Game {
        Game {
            id: id.into(),
            sport: Sport::Nfl,
            home: "Chiefs".into(),
            away: "Raiders".into(),
            starts_at: "2026-09-10T20:15:00Z".into(),
            spread_lines: lines
                .into_iter()
                .enumerate()
                .map(|(i, l)| BookLine {
                    book: format!("book{i}"),
                    american_odds: -110,
                    line: Some(l),
                    fetched_at: "2026-09-10T18:00:00Z".into(),
                })
                .collect(),
            moneyline_lines: vec![],
            total_lines: vec![],
        }
    }

    fn history(points: &[f64]) -> HashMap<String, Vec<LinePoint>> {
        let mut m = HashMap::new();
        m.insert(
            "g1".to_string(),
            points
                .iter()
                .enumerate()
                .map(|(i, line)| LinePoint { at: format!("t{i}"), line: *line })
                .collect(),
        );
        m
    }

    #[test]
    fn needs_consensus_of_three_books() {
        assert!(run_model(&[game_with_lines("g1", vec![-4.0, -4.5])], &HashMap::new()).is_empty());
    }

    #[test]
    fn deviation_below_half_a_point_is_not_an_edge() {
        // fair = -4.2, best deviation 0.2 < MIN_EDGE → no pick.
        let out = run_model(&[game_with_lines("g1", vec![-4.0, -4.2, -4.4])], &HashMap::new());
        assert!(out.is_empty());
    }

    #[test]
    fn book_shading_toward_away_is_an_away_pick() {
        // fair = -5.0; -10 deviates most (away side gets +10 vs fair +5).
        let out = run_model(&[game_with_lines("g1", vec![-10.0, -4.0, -5.0])], &HashMap::new());
        assert_eq!(out.len(), 1);
        let mo = &out[0];
        assert_eq!(mo.pick_team, PickTeam::Away);
        assert_eq!(mo.side, "Raiders +10");
        assert_eq!(mo.picked_line, -10.0);
        assert_eq!(mo.fair_line, -5.0);
        assert_eq!(mo.edge_pct, 5.0);
        assert_eq!(mo.confidence, 4); // edge >= 2.0
        assert_eq!(mo.suggested_units, 2.5); // round(5 * 0.25 * 2 * 10)/10
    }

    #[test]
    fn book_shading_toward_home_is_a_home_pick() {
        // fair = -4.0; -2 is the outlier — home bettors lay only 2.
        let out = run_model(&[game_with_lines("g1", vec![-2.0, -4.0, -5.0])], &HashMap::new());
        assert_eq!(out.len(), 1);
        let mo = &out[0];
        assert_eq!(mo.pick_team, PickTeam::Home);
        assert_eq!(mo.side, "Chiefs -2");
        assert_eq!(mo.edge_pct, 2.0);
        assert_eq!(mo.suggested_units, 1.0);
    }

    #[test]
    fn units_are_capped_at_max_units() {
        // edge 7 → raw sizing 3.5u → capped to 3.0u.
        let out = run_model(&[game_with_lines("g1", vec![-12.0, -4.0, -5.0])], &HashMap::new());
        assert_eq!(out[0].edge_pct, 7.0);
        assert_eq!(out[0].suggested_units, MAX_UNITS);
    }

    #[test]
    fn steam_toward_home_confirms_a_home_pick() {
        // Median moved -5.0 → -6.5 (money on home) and we picked home.
        let g = game_with_lines("g1", vec![-2.0, -5.0, -6.0]); // fair -5, home pick, base conf 4
        let out = run_model(&[g], &history(&[-5.0, -6.5]));
        assert_eq!(out[0].confidence, 5);
        assert!(out[0].factors.iter().any(|f| f.name == "line steam" && f.direction == FactorDirection::For));
    }

    #[test]
    fn steam_against_the_pick_downgrades_confidence() {
        // Same home-side steam, but the outlier puts us on the away side.
        let g = game_with_lines("g1", vec![-8.0, -5.0, -4.0]); // fair -5, away pick, base conf 4
        let out = run_model(&[g], &history(&[-5.0, -6.5]));
        assert_eq!(out[0].confidence, 3);
        assert!(out[0].factors.iter().any(|f| f.name == "line steam" && f.direction == FactorDirection::Against));
    }

    #[test]
    fn sub_threshold_movement_is_not_steam() {
        let g = game_with_lines("g1", vec![-2.0, -5.0, -6.0]);
        let out = run_model(&[g], &history(&[-5.0, -5.5])); // |movement| < STEAM_THRESHOLD
        assert_eq!(out[0].confidence, 4);
        assert!(!out[0].factors.iter().any(|f| f.name == "line steam"));
    }

    #[test]
    fn outputs_are_sorted_best_edge_first() {
        let strong = game_with_lines("strong", vec![-12.0, -4.0, -5.0]); // edge 7
        let weak = game_with_lines("weak", vec![-3.0, -4.0, -5.0]); // edge 1
        let out = run_model(&[weak, strong], &HashMap::new());
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].edge_pct, out[1].edge_pct), (7.0, 1.0));
    }

    #[test]
    fn empty_history_snapshots_the_fetched_line() {
        let out = run_model(&[game_with_lines("g1", vec![-2.0, -4.0, -5.0])], &HashMap::new());
        assert_eq!(out[0].line_history.len(), 1);
        assert_eq!(out[0].line_history[0].line, -2.0);
    }

    #[test]
    fn bankroll_scaling_rounds_to_cents() {
        // 1 unit = 1% of bankroll: 2 units at $1,000 → $20.00.
        assert_eq!(scale_units_to_bankroll(2.0, 1000.0), 20.0);
        assert_eq!(scale_units_to_bankroll(1.5, 333.33), 5.0); // 499.995 → rounds to cents
    }
}
