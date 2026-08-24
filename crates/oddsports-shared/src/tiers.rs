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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_orders_and_gates_correctly() {
        assert!(Tier::Free < Tier::Starter && Tier::Starter < Tier::Analyst && Tier::Analyst < Tier::Sharp);
        assert!(Tier::Analyst.can_access(Tier::Starter));
        assert!(!Tier::Starter.can_access(Tier::Analyst));
        assert!(Tier::Sharp.can_access(Tier::Sharp));
    }

    #[test]
    fn next_walks_the_ladder_and_stops_at_sharp() {
        let mut tier = Tier::Free;
        let mut steps = 0;
        while let Some(next) = tier.next() {
            tier = next;
            steps += 1;
            assert!(steps <= 3);
        }
        assert_eq!((steps, tier), (3, Tier::Sharp));
    }

    #[test]
    fn beehiiv_names_are_case_insensitive_and_known_names_map_off_free() {
        // Known names must never degrade a paying subscriber to Free.
        for name in ["starter", "STARTER", "analyst", "Sharp"] {
            assert_ne!(Tier::from_beehiiv_name(name), Tier::Free, "case handling broke for {name}");
        }
        assert_eq!(Tier::from_beehiiv_name("starter"), Tier::Starter);
        assert_eq!(Tier::from_beehiiv_name("SHARP"), Tier::Sharp);
        // Unknown → Free is the safe direction: degrade, never upgrade silently.
        assert_eq!(Tier::from_beehiiv_name("mystery-tier"), Tier::Free);
        assert_eq!(Tier::from_beehiiv_name(""), Tier::Free);
    }

    #[test]
    fn paid_tiers_have_prices_free_does_not() {
        assert_eq!(Tier::Free.price_usd(), None);
        assert!(Tier::Starter.price_usd().unwrap() < Tier::Analyst.price_usd().unwrap());
        assert!(Tier::Analyst.price_usd().unwrap() < Tier::Sharp.price_usd().unwrap());
    }
}
