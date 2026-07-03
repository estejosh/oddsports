//! Smoke test for the BTCPC inference path: builds a synthetic game and
//! model output, then generates prose at every tier depth.
//!
//!     cargo run -p oddsports-pipeline --example prose_smoke

use oddsports_pipeline::budget::TokenBudget;
use oddsports_pipeline::prose::write_pick_prose;
use oddsports_shared::{
    Factor, FactorDirection, Game, MarketType, ModelOutput, PickTeam, Sport, Tier,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let game = Game {
        id: "smoke-1".into(),
        sport: Sport::Nba,
        home: "Celtics".into(),
        away: "Knicks".into(),
        starts_at: "2026-07-03T23:30:00Z".into(),
        spread_lines: vec![],
        moneyline_lines: vec![],
        total_lines: vec![],
    };
    let model_out = ModelOutput {
        game_id: game.id.clone(),
        market: MarketType::Spread,
        side: "Celtics -4.5".into(),
        pick_team: PickTeam::Home,
        picked_line: -4.5,
        fair_line: -6.0,
        edge_pct: 3.2,
        confidence: 3,
        suggested_units: 1.0,
        factors: vec![
            Factor {
                name: "consensus deviation".into(),
                direction: FactorDirection::For,
                weight: 0.6,
                detail: "Book consensus sits 1.5 pts short of fair line".into(),
            },
            Factor {
                name: "rest advantage".into(),
                direction: FactorDirection::For,
                weight: 0.4,
                detail: "Home side on 2 days rest vs back-to-back".into(),
            },
        ],
        line_history: vec![],
    };

    let client = reqwest::Client::new();
    let mut budget = TokenBudget::from_env();

    for tier in [Tier::Free, Tier::Starter, Tier::Sharp] {
        let started = std::time::Instant::now();
        let res = write_pick_prose(&client, &game, &model_out, tier, &mut budget).await?;
        println!(
            "--- {tier:?} ({} in / {} out tokens, {:.1}s) ---\n{}\n",
            res.input_tokens,
            res.output_tokens,
            started.elapsed().as_secs_f32(),
            res.text
        );
    }
    Ok(())
}
