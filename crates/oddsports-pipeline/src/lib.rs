pub mod budget;
pub mod grading;
pub mod ingest;
pub mod model;
pub mod prose;
pub mod store;

use anyhow::Result;
use chrono::{Duration, NaiveDate, Utc};
use oddsports_shared::{
    assert_compliant, available_books, compliance_footer, tracked_link, BookRef, DailySlate,
    GenerationStats, LinkContext, PickBlock, Sport, Tier,
};

const FREE_PICK_COUNT: usize = 5;

/// Daily pipeline orchestrator. PRD Section 6 #1: ONE generation pass per day.
/// data → deterministic model → tiered LLM prose → compliance check → cache + send.
/// Marginal cost per SUBSCRIBER is zero — cost scales with games.
pub async fn run_daily_pipeline(date: NaiveDate) -> Result<DailySlate> {
    let date_str = date.format("%Y-%m-%d").to_string();
    let db = store::open_db()?;
    let mut budget = budget::TokenBudget::from_env();
    let client = reqwest::Client::new();

    // Step 0: grade yesterday and build "The Record" reveal (PRD P0 #11).
    // Zero AI tokens — pure data. Reveal shows full paid-tier depth to ALL tiers.
    let yesterday = (date - Duration::days(1)).format("%Y-%m-%d").to_string();
    grading::grade_slate(&db, &yesterday).await?;
    let reveal = grading::build_reveal_post(&db, &yesterday)?;
    tracing::info!(chars = reveal.len(), "reveal ready");
    // TODO(fable): publish reveal to free Telegram channel + top of today's
    // email for every tier. Immutable once posted — never edit past reveals.

    tracing::info!(date = %date_str, "ingesting");
    let games = ingest::fetch_games(Sport::LAUNCH).await?;
    tracing::info!(games = games.len(), "fetched");

    // Persist this fetch as a snapshot batch, then hand the model each game's
    // full snapshot history (steam signal + honest line_history for /line).
    store::save_line_snapshots(&db, &games)?;
    let mut history = std::collections::HashMap::new();
    for g in &games {
        history.insert(g.id.clone(), store::line_history(&db, &g.id)?);
    }

    let model_outputs = model::run_model(&games, &history); // sorted best-edge first
    tracing::info!(edges = model_outputs.len(), "actionable edges");

    let mut picks = Vec::new();
    let mut stats = GenerationStats::default();

    for (i, mo) in model_outputs.iter().enumerate() {
        let game = games.iter().find(|g| g.id == mo.game_id).expect("game exists");

        // Tier depths to render. Free tier only gets top N picks.
        let tiers: &[Tier] = if i < FREE_PICK_COUNT {
            &[Tier::Free, Tier::Starter, Tier::Analyst]
        } else {
            &[Tier::Starter, Tier::Analyst]
        };

        for &tier in tiers {
            let prose = prose::write_pick_prose(&client, game, mo, tier, &mut budget).await?;
            stats.input_tokens += prose.input_tokens;
            stats.output_tokens += prose.output_tokens;

            let links: Vec<BookRef> = available_books()
                .iter()
                .filter_map(|book| {
                    tracked_link(
                        book,
                        &LinkContext {
                            surface: "email",
                            tier: tier.name(),
                            game_id: Some(&game.id),
                            subscriber_ref: None,
                        },
                    )
                    .map(|url| BookRef { book: book.to_string(), url })
                })
                .collect();

            let stars = "★".repeat(mo.confidence as usize) + &"☆".repeat(5 - mo.confidence as usize);
            let sizing = if tier >= Tier::Analyst {
                format!("\n\nSuggested size: {}u", mo.suggested_units)
            } else {
                String::new()
            };
            let link_row = links
                .iter()
                .map(|l| format!("[{}]({})", l.book, l.url))
                .collect::<Vec<_>>()
                .join(" · ");

            let body = format!(
                "**{} @ {}** — {}\n\nConfidence: {stars}\n\n{}{sizing}\n\nBet at: {link_row}{}",
                game.away,
                game.home,
                mo.side,
                prose.text,
                compliance_footer()
            );

            // Template lock — errors abort the run if compliance blocks missing.
            assert_compliant(&body)?;

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
                links,
            });
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

/// Lightweight snapshot pass — fetch current lines, persist, exit.
/// Run every 15–30 min via cron/systemd timer. Zero AI cost; this is the
/// data source for steam detection and closing-line (CLV) capture.
pub async fn run_snapshot() -> Result<usize> {
    let db = store::open_db()?;
    let games = ingest::fetch_games(Sport::LAUNCH).await?;
    let rows = store::save_line_snapshots(&db, &games)?;
    tracing::info!(games = games.len(), rows, "snapshot saved");
    Ok(rows)
}
