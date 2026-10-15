//! Domain types shared by pipeline and bot.

use crate::tiers::Tier;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sport {
    Nfl,
    Nba,
    Mlb,
    Nhl,
    SoccerEpl,
    SoccerUcl,
    Mma,
    Boxing,
}

impl Sport {
    pub const LAUNCH: &'static [Sport] = &[
        Sport::Nfl,
        Sport::Nba,
        Sport::Mlb,
        Sport::Nhl,
        Sport::SoccerEpl,
        Sport::SoccerUcl,
        Sport::Mma,
        Sport::Boxing,
    ];

    /// the-odds-api.com sport keys.
    pub fn odds_api_key(&self) -> &'static str {
        match self {
            Sport::Nfl => "americanfootball_nfl",
            Sport::Nba => "basketball_nba",
            Sport::Mlb => "baseball_mlb",
            Sport::Nhl => "icehockey_nhl",
            Sport::SoccerEpl => "soccer_epl",
            Sport::SoccerUcl => "soccer_uefa_champs_league",
            Sport::Mma => "mma_mixed_martial_arts",
            Sport::Boxing => "boxing_boxing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketType {
    Spread,
    Moneyline,
    Total,
    Prop,
    Parlay,
}

/// One book's price on one market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookLine {
    pub book: String,
    pub american_odds: i32,
    pub line: Option<f64>,
    /// Which side this price is for (team name, "Over", ...). Needed to remove
    /// the vig from two-way markets. Absent on rows stored before this field existed.
    #[serde(default)]
    pub outcome: Option<String>,
    pub fetched_at: String, // ISO
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub sport: Sport,
    pub home: String,
    pub away: String,
    pub starts_at: String, // ISO
    pub spread_lines: Vec<BookLine>,
    pub moneyline_lines: Vec<BookLine>,
    pub total_lines: Vec<BookLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Factor {
    pub name: String,
    pub direction: FactorDirection,
    pub weight: f64,
    pub detail: String,
    /// Where the claim comes from: an http(s) URL, or `internal:<name>` for
    /// factors computed by our own code. A factor with no source is dropped.
    #[serde(default)]
    pub source: String,
    /// RFC 3339 timestamp the source data was current as of.
    #[serde(default)]
    pub as_of: String,
}

impl Factor {
    /// A factor is publishable only if it names a source (http(s) URL, or
    /// `internal:<name>` for our own computed signals) and a timestamp.
    pub fn is_sourced(&self) -> bool {
        let s = self.source.trim();
        let ok_source = s.starts_with("https://")
            || s.starts_with("http://")
            || (s.starts_with("internal:") && s.len() > "internal:".len());
        ok_source && !self.as_of.trim().is_empty()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactorDirection {
    For,
    Against,
}

/// Deterministic model output — produced by code, never by an LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelOutput {
    pub game_id: String,
    pub market: MarketType,
    /// Human-readable side, e.g. "Chiefs -3.5".
    pub side: String,
    /// Structured settlement info — grading never parses strings.
    pub pick_team: PickTeam,
    pub picked_line: f64,
    pub fair_line: f64,
    pub edge_pct: f64,
    pub confidence: u8, // 1..=5
    pub suggested_units: f64,
    /// LLM turns these into prose, never invents its own.
    pub factors: Vec<Factor>,
    pub line_history: Vec<LinePoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PickTeam {
    Home,
    Away,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinePoint {
    pub at: String,
    pub line: f64,
}

/// A pick as rendered content — one per game per tier depth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickBlock {
    pub game_id: String,
    pub sport: Sport,
    /// "Away @ Home" — used for fuzzy game matching in bot commands.
    pub matchup: String,
    pub min_tier: Tier,
    /// Rendered markdown for this tier depth. Includes compliance footer.
    pub body: String,
    pub confidence: u8,
    pub risk_warning: String,
    /// Structured copy of the model output for grading/raw display.
    pub model: ModelOutput,
}

/// One day's fully generated content — the unit the pipeline produces once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailySlate {
    pub date: String, // YYYY-MM-DD
    pub picks: Vec<PickBlock>,
    pub generation: GenerationStats,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GenerationStats {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscriber {
    pub beehiiv_id: String,
    pub email: String,
    pub tier: Tier,
    pub telegram_user_id: Option<i64>,
    pub bankroll_usd: Option<f64>,
    pub linked_at: Option<String>,
}
