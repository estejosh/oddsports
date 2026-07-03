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
