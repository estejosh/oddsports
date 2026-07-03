//! Tier ladder — the spine of the product. PRD Section 4.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum Tier {
    Free = 0,
    Starter = 1,
    Analyst = 2,
    Sharp = 3,
}

impl Tier {
    pub fn from_u8(n: u8) -> Self {
        match n {
            1 => Tier::Starter,
            2 => Tier::Analyst,
            3 => Tier::Sharp,
            _ => Tier::Free,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Tier::Free => "Free",
            Tier::Starter => "Starter",
            Tier::Analyst => "Analyst",
            Tier::Sharp => "Sharp",
        }
    }

    /// USD/month. None = free. Final pricing pending finance (PRD open question).
    pub fn price_usd(&self) -> Option<u32> {
        match self {
            Tier::Free => None,
            Tier::Starter => Some(19),
            Tier::Analyst => Some(49),
            Tier::Sharp => Some(129),
        }
    }

    /// What this tier unlocks — used for upgrade prompts.
    pub fn unlocks(&self) -> &'static [&'static str] {
        match self {
            Tier::Free => &["Top 3–5 daily picks", "Confidence stars", "Odds comparison"],
            Tier::Starter => &["Full daily slate", "Form / H2H / injury notes", "Private Telegram channel"],
            Tier::Analyst => &[
                "Model factor breakdowns",
                "Line movement & steam tracking",
                "Props/parlays with correlation notes",
                "Suggested unit sizing",
                "/why /line /units bot commands",
            ],
            Tier::Sharp => &[
                "Live in-game alerts",
                "Raw model output",
                "Personalized bankroll pacing",
                "Weekly office-hours recap",
                "Earliest delivery",
            ],
        }
    }

    /// True if a subscriber at `self` may see content gated at `need`.
    pub fn can_access(&self, need: Tier) -> bool {
        *self >= need
    }

    /// Next tier up, for upgrade CTAs. None at top of ladder.
    pub fn next(&self) -> Option<Tier> {
        match self {
            Tier::Free => Some(Tier::Starter),
            Tier::Starter => Some(Tier::Analyst),
            Tier::Analyst => Some(Tier::Sharp),
            Tier::Sharp => None,
        }
    }

    /// Map Beehiiv subscription tier names → Tier. Must match Beehiiv setup.
    pub fn from_beehiiv_name(name: &str) -> Tier {
        match name.to_ascii_lowercase().as_str() {
            "starter" => Tier::Starter,
            "analyst" => Tier::Analyst,
            "sharp" => Tier::Sharp,
            _ => Tier::Free,
        }
    }
}
