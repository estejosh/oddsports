//! Data ingestion — odds, lines, schedules. No AI here.
//! v1 target: the-odds-api.com (cheap, covers all launch sports).
//! TODO(fable): swap in Betchu's own odds feed when available (PRD open question).

use anyhow::Result;
use chrono::Utc;
use oddsports_shared::{BookLine, Game, Sport};
use serde::Deserialize;
use std::env;

#[derive(Deserialize)]
struct ApiEvent {
    id: String,
    home_team: String,
    away_team: String,
    commence_time: String,
    #[serde(default)]
    bookmakers: Vec<ApiBookmaker>,
}

#[derive(Deserialize)]
struct ApiBookmaker {
    key: String,
    #[serde(default)]
    markets: Vec<ApiMarket>,
}

#[derive(Deserialize)]
struct ApiMarket {
    key: String,
    #[serde(default)]
    outcomes: Vec<ApiOutcome>,
}

#[derive(Deserialize)]
struct ApiOutcome {
    price: f64,
    point: Option<f64>,
}

pub async fn fetch_games(sports: &[Sport]) -> Result<Vec<Game>> {
    let (Ok(base), Ok(key)) = (env::var("ODDS_API_BASE"), env::var("ODDS_API_KEY")) else {
        tracing::warn!("ODDS_API_* not configured — returning empty slate");
        return Ok(vec![]);
    };

    let client = reqwest::Client::new();
    let mut games = Vec::new();

    for sport in sports {
        let url = format!(
            "{base}/sports/{}/odds?regions=us&markets=h2h,spreads,totals&oddsFormat=american&apiKey={key}",
            sport.odds_api_key()
        );
        let res = client.get(&url).send().await?;
        if !res.status().is_success() {
            tracing::warn!(sport = ?sport, status = %res.status(), "skipping sport");
            continue;
        }
        let events: Vec<ApiEvent> = res.json().await?;
        for ev in events {
            games.push(normalize(*sport, ev));
        }
    }
    Ok(games)
}

fn normalize(sport: Sport, ev: ApiEvent) -> Game {
    let now = Utc::now().to_rfc3339();
    let mut spread = Vec::new();
    let mut moneyline = Vec::new();
    let mut total = Vec::new();

    for bm in &ev.bookmakers {
        for market in &bm.markets {
            let dest = match market.key.as_str() {
                "spreads" => &mut spread,
                "h2h" => &mut moneyline,
                "totals" => &mut total,
                _ => continue,
            };
            for o in &market.outcomes {
                dest.push(BookLine {
                    book: bm.key.clone(),
                    american_odds: o.price as i32,
                    line: o.point,
                    fetched_at: now.clone(),
                });
            }
        }
    }

    Game {
        id: ev.id,
        sport,
        home: ev.home_team,
        away: ev.away_team,
        starts_at: ev.commence_time,
        spread_lines: spread,
        moneyline_lines: moneyline,
        total_lines: total,
    }
}
