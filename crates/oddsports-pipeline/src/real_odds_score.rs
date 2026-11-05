//! Stage 5 — real odds score. Combines the market-derived `OddsScore` with
//! SOURCED factors (injuries, lineup news, ...). Pure math.
//!
//! v1 heuristic, deliberately small and bounded: each sourced factor nudges
//! the home team's fair probability by `weight * PP_PER_WEIGHT` percentage
//! points (For = toward home, Against = toward away), total capped at
//! ±`MAX_ADJ_PP`. Unsourced factors are ignored. With no factors the result is
//! identical to the odds score.
//!
//! Direction convention for factors fed to this stage: relative to the HOME team.

use crate::odds_score::OddsScore;
use oddsports_shared::{Factor, FactorDirection};

pub const PP_PER_WEIGHT: f64 = 0.5;
pub const MAX_ADJ_PP: f64 = 2.0;

#[derive(Debug, Clone, PartialEq)]
pub struct RealOddsScore {
    pub home_prob: f64,
    pub away_prob: f64,
    pub draw_prob: Option<f64>,
    /// Applied nudge to the home probability, in percentage points.
    pub adjustment_pp: f64,
    pub factors_applied: usize,
    pub base: OddsScore,
}

pub fn apply(base: &OddsScore, factors: &[Factor]) -> RealOddsScore {
    let sourced: Vec<&Factor> = factors.iter().filter(|f| f.is_sourced()).collect();
    let raw: f64 = sourced
        .iter()
        .map(|f| {
            let w = if f.weight.is_finite() { f.weight.clamp(0.0, 1.0) } else { 0.0 };
            match f.direction {
                FactorDirection::For => w,
                FactorDirection::Against => -w,
            }
        })
        .sum::<f64>()
        * PP_PER_WEIGHT;
    let adj_pp = raw.clamp(-MAX_ADJ_PP, MAX_ADJ_PP);

    let new_home = (base.home_prob + adj_pp / 100.0).clamp(0.01, 0.99);
    let old_rest = 1.0 - base.home_prob;
    let scale = if old_rest > 0.0 { (1.0 - new_home) / old_rest } else { 1.0 };

    RealOddsScore {
        home_prob: new_home,
        away_prob: base.away_prob * scale,
        draw_prob: base.draw_prob.map(|d| d * scale),
        adjustment_pp: (new_home - base.home_prob) * 100.0,
        factors_applied: sourced.len(),
        base: base.clone(),
    }
}

impl RealOddsScore {
    pub fn fair_american(&self) -> Option<(i32, i32)> {
        use crate::aggregate::prob_to_american;
        Some((prob_to_american(self.home_prob)?, prob_to_american(self.away_prob)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> OddsScore {
        OddsScore {
            game_id: "g".into(),
            home: "H".into(),
            away: "A".into(),
            home_prob: 0.60,
            away_prob: 0.40,
            draw_prob: None,
            books: 6,
            dispersion_pp: 0.4,
            confidence: 5,
            home_edge_pp: 0.0,
            away_edge_pp: 0.0,
        }
    }

    fn factor(dir: FactorDirection, weight: f64, sourced: bool) -> Factor {
        Factor {
            name: "n".into(),
            direction: dir,
            weight,
            detail: "d".into(),
            source: if sourced { "https://example.org/report".into() } else { String::new() },
            as_of: if sourced { "2026-10-01T12:00:00Z".into() } else { String::new() },
        }
    }

    #[test]
    fn no_factors_is_identity() {
        let r = apply(&base(), &[]);
        assert_eq!(r.home_prob, 0.60);
        assert_eq!(r.adjustment_pp, 0.0);
        assert_eq!(r.factors_applied, 0);
    }

    #[test]
    fn unsourced_factor_is_ignored() {
        let r = apply(&base(), &[factor(FactorDirection::For, 1.0, false)]);
        assert_eq!(r.home_prob, 0.60);
        assert_eq!(r.factors_applied, 0);
    }

    #[test]
    fn sourced_for_factor_raises_home_and_probabilities_still_sum_to_one() {
        let r = apply(&base(), &[factor(FactorDirection::For, 1.0, true)]);
        assert!((r.adjustment_pp - 0.5).abs() < 1e-9);
        assert!(r.home_prob > 0.60);
        assert!((r.home_prob + r.away_prob - 1.0).abs() < 1e-9);
    }

    #[test]
    fn against_factor_lowers_home() {
        let r = apply(&base(), &[factor(FactorDirection::Against, 1.0, true)]);
        assert!(r.home_prob < 0.60);
    }

    #[test]
    fn adjustment_is_capped() {
        let many: Vec<_> = (0..50).map(|_| factor(FactorDirection::For, 1.0, true)).collect();
        let r = apply(&base(), &many);
        assert!((r.adjustment_pp - MAX_ADJ_PP).abs() < 1e-9);
    }

    #[test]
    fn hostile_weights_are_neutralised() {
        let r = apply(
            &base(),
            &[factor(FactorDirection::For, f64::NAN, true), factor(FactorDirection::For, 1e9, true)],
        );
        assert!(r.home_prob.is_finite());
        assert!(r.adjustment_pp <= 0.5 + 1e-9, "weight clamps to 1.0 => 0.5pp");
    }

    #[test]
    fn three_way_probabilities_still_sum_to_one() {
        let mut b = base();
        b.home_prob = 0.45;
        b.away_prob = 0.30;
        b.draw_prob = Some(0.25);
        let r = apply(&b, &[factor(FactorDirection::For, 1.0, true)]);
        let sum = r.home_prob + r.away_prob + r.draw_prob.unwrap();
        assert!((sum - 1.0).abs() < 1e-9);
    }
}
