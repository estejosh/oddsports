//! OddSports pipeline — a vendor-neutral odds-analysis engine.
//!
//! Stages, in order (each is its own module; math stages are pure and tested):
//!
//!   1. `sources`          fetch odds/lines from the configured feed
//!   2. `aggregate`        no-vig consensus + dispersion across books
//!   3. `odds_score`       our fair odds, confidence, edge vs best price
//!   4. `ai_factors`       sourced factors extracted from supplied documents
//!   5. `real_odds_score`  fair odds nudged (bounded) by sourced factors
//!   6. `publish`          render + hard gates (no per-source odds, compliance)
//!   7. track record       `grading` (immutable record) + `calibration` (Brier,
//!                         log loss, CLV)
//!
//! `model` (spread-edge picks), `prose` (LLM wording), `budget` and `store`
//! support those stages. Nothing in this crate promotes a sportsbook, operator
//! or affiliate program; there is no CTA and no referral code.

pub mod aggregate;
pub mod ai_config;
pub mod ai_factors;
pub mod budget;
pub mod calibration;
pub mod factor_docs;
pub mod forecasts;
pub mod grading;
pub mod maintenance;
pub mod model;
pub mod odds_score;
pub mod prose;
pub mod publish;
pub mod real_odds_score;
pub mod scores;
pub mod sources;
pub mod store;

use anyhow::Result;
use chrono::{NaiveDate, Utc};
use oddsports_shared::{DailySlate, GenerationStats, PickBlock, Sport, Templates, Tier};
use std::collections::HashMap;

const FREE_PICK_COUNT: usize = 5;

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

/// Daily pipeline orchestrator. PRD Section 6 #1: ONE generation pass per day.
/// Marginal cost per SUBSCRIBER is zero — cost scales with games.
pub async fn run_daily_pipeline(date: NaiveDate) -> Result<DailySlate> {
    let date_str = date.format("%Y-%m-%d").to_string();
    let db = store::open_db()?;
    let mut budget = budget::TokenBudget::from_env();
    let templates = Templates::from_env();
    let ai_cfg = ai_config::AiConfig::from_env();
    let ai_factors_on = env_flag("AI_FACTORS_ENABLED");
    // CPU inference on a busy node can be slow but must never hang the run.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    // Stage 7 (first, so yesterday is settled): grade EVERY past slate (PRD P0 #11).
    // Idempotent per pick, zero AI tokens.
    let past_dates = run_grading(&db, &date_str).await?;
    // Reveal covers the most recent past slate (usually yesterday). Shows
    // full paid-tier depth to ALL tiers.
    let reveal = match past_dates.last() {
        Some(d) => grading::build_reveal_post(&db, d)?,
        None => String::new(),
    };
    tracing::info!(chars = reveal.len(), "reveal ready");
    // TODO(fable): publish reveal to free Telegram channel + top of today's
    // email for every tier. Immutable once posted — never edit past reveals.

    // Stage 1: sources
    tracing::info!(date = %date_str, "fetching sources");
    let games = sources::fetch_games(Sport::LAUNCH).await?;
    tracing::info!(games = games.len(), "fetched");

    // Persist this fetch as a snapshot batch, then hand the model each game's
    // full snapshot history (steam signal + honest line_history for /line).
    store::save_line_snapshots(&db, &games)?;
    let mut history = HashMap::new();
    for g in &games {
        history.insert(g.id.clone(), store::line_history(&db, &g.id)?);
    }

    // Stages 2-3: consensus + fair odds per game (pure math).
    let odds_scores: HashMap<String, odds_score::OddsScore> = games
        .iter()
        .filter_map(|g| odds_score::score_game(g).map(|s| (g.id.clone(), s)))
        .collect();
    tracing::info!(scored = odds_scores.len(), "fair odds computed");
    // Forecast ledger: the home-win probability we publish, per game. Starts
    // at the market-derived score; games that get sourced factors are
    // upgraded to the real odds score in the loop below.
    let mut final_probs: HashMap<String, f64> =
        odds_scores.iter().map(|(id, s)| (id.clone(), s.home_prob)).collect();

    // Refresh sourced documents for AI factors (no-op unless enabled + configured).
    if ai_factors_on {
        factor_docs::refresh(&client, &games).await;
    }

    let model_outputs = model::run_model(&games, &history); // sorted best-edge first
    // Prose is the cost + latency driver (calls = picks × tier depths), so the
    // slate takes only the best N edges. Raise the cap as inference throughput allows.
    let max_picks: usize = std::env::var("MAX_PICKS_PER_DAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let taking = model_outputs.len().min(max_picks);
    tracing::info!(edges = model_outputs.len(), taking, "actionable edges");

    let mut picks = Vec::new();
    let mut stats = GenerationStats::default();

    for (i, mo) in model_outputs.iter().take(max_picks).enumerate() {
        let game = games.iter().find(|g| g.id == mo.game_id).expect("game exists");

        // Stage 4: sourced AI factors (off unless enabled AND documents exist).
        let mut ai_factors = Vec::new();
        if ai_factors_on {
            let docs = ai_factors::load_context_docs(&game.id);
            match ai_factors::fetch_ai_factors(&client, &ai_cfg, game, &docs).await {
                Ok((v, tin, tout)) => {
                    stats.input_tokens += tin;
                    stats.output_tokens += tout;
                    for why in &v.rejected {
                        tracing::warn!(game = %game.id, %why, "ai factor dropped");
                    }
                    ai_factors = v.kept;
                }
                Err(e) => tracing::warn!(game = %game.id, error = %e, "ai factors unavailable"),
            }
        }

        // Stage 5: fair odds nudged by sourced factors.
        let fair = odds_scores
            .get(&game.id)
            .map(|s| real_odds_score::apply(s, &ai_factors));
        if let Some(f) = &fair {
            final_probs.insert(game.id.clone(), f.home_prob);
        }

        // The LLM sees model factors plus the sourced AI factors (home-relative).
        let mut mo_for_prose = mo.clone();
        mo_for_prose.factors.extend(ai_factors.iter().map(|f| {
            let mut f = f.clone();
            f.name = format!("{} (relative to {})", f.name, game.home);
            f
        }));
        mo_for_prose.factors.retain(|f| f.is_sourced());

        // Tier depths to render. Free tier only gets top N picks.
        let tiers: &[Tier] = if i < FREE_PICK_COUNT {
            &[Tier::Free, Tier::Starter, Tier::Analyst]
        } else {
            &[Tier::Starter, Tier::Analyst]
        };

        for &tier in tiers {
            let started = std::time::Instant::now();
            let prose = prose::write_pick_prose(&client, game, &mo_for_prose, tier, &mut budget).await?;
            tracing::info!(
                pick = i + 1,
                taking,
                tier = ?tier,
                game = %mo.side,
                secs = started.elapsed().as_secs(),
                "prose done"
            );
            stats.input_tokens += prose.input_tokens;
            stats.output_tokens += prose.output_tokens;

            // Stage 6: render + hard gates. If the LLM's prose names a book,
            // swap in the deterministic template rather than ship or abort.
            let body = match publish::render_pick(&templates, game, mo, tier, &prose.text, fair.as_ref()) {
                Ok(b) => b,
                Err(e) if e.to_string().contains("SOURCE LEAK") => {
                    tracing::warn!(game = %game.id, error = %e, "prose named a book — template fallback");
                    let fb = prose::template_fallback(mo);
                    publish::render_pick(&templates, game, mo, tier, &fb.text, fair.as_ref())?
                }
                Err(e) => return Err(e), // compliance block aborts the run
            };

            picks.push(PickBlock {
                game_id: game.id.clone(),
                sport: game.sport,
                matchup: format!("{} @ {}", game.away, game.home),
                min_tier: tier,
                body,
                confidence: mo.confidence,
                risk_warning: if mo.edge_pct >= 2.0 {
                    "Large line deviation — verify news (injury/lineup) before betting.".into()
                } else {
                    "Standard variance applies — size responsibly.".into()
                },
                model: mo.clone(),
            });
        }
    }

    for g in &games {
        if let Some(p) = final_probs.get(&g.id) {
            forecasts::record(&db, &date_str, g, *p)?;
        }
    }

    stats.cost_usd = budget.spent();
    let slate = DailySlate { date: date_str.clone(), picks, generation: stats };
    store::save_slate(&db, &slate)?;
    tracing::info!(
        blocks = slate.picks.len(),
        cost_usd = slate.generation.cost_usd,
        "pipeline done"
    );

    // TODO(fable): push to Beehiiv via API (draft post per tier) — human review
    // gate stays ON at launch (PRD open question: content ops).

    Ok(slate)
}

pub fn today() -> NaiveDate {
    Utc::now().date_naive()
}

/// Grade every slate before `before` (idempotent). Returns the dates
/// considered, ascending. Callable standalone via the `grade` subcommand —
/// catch up on grading without regenerating (and re-paying for) a slate.
pub async fn run_grading(db: &rusqlite::Connection, before: &str) -> Result<Vec<String>> {
    let past_dates = store::slate_dates_before(db, before)?;
    for d in &past_dates {
        grading::grade_slate(db, d).await?;
        forecasts::settle(db, d).await?;
    }
    Ok(past_dates)
}

/// Lightweight snapshot pass — fetch current lines, persist, exit.
/// Run every 15–30 min via cron/systemd timer. Zero AI cost; this is the
/// data source for steam detection and closing-line (CLV) capture.
pub async fn run_snapshot() -> Result<usize> {
    let db = store::open_db()?;
    let games = sources::fetch_games(Sport::LAUNCH).await?;
    let rows = store::save_line_snapshots(&db, &games)?;
    tracing::info!(games = games.len(), rows, "snapshot saved");
    Ok(rows)
}
