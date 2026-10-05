//! Track-record scoring. Pure math. Measures whether our probabilities are
//! honest, not just whether picks won:
//!   - Brier score  (mean squared error of probabilities; lower is better, 0.25 = coin flip)
//!   - log loss     (punishes confident misses hard; lower is better, ln 2 ≈ 0.693 = coin flip)
//!   - CLV          (did our price beat the closing price?)

/// One graded forecast: the probability we gave for an event, and whether it happened.
#[derive(Debug, Clone, Copy)]
pub struct Forecast {
    pub prob: f64,
    pub happened: bool,
}

/// Mean squared error between forecast probability and outcome. `None` if empty.
pub fn brier_score(fs: &[Forecast]) -> Option<f64> {
    if fs.is_empty() {
        return None;
    }
    let sum: f64 = fs
        .iter()
        .map(|f| {
            let y = if f.happened { 1.0 } else { 0.0 };
            (f.prob - y).powi(2)
        })
        .sum();
    Some(sum / fs.len() as f64)
}

/// Negative mean log-likelihood. Probabilities are clamped to [EPS, 1-EPS] so a
/// single 0%/100% forecast gives a large finite penalty instead of infinity.
pub fn log_loss(fs: &[Forecast]) -> Option<f64> {
    const EPS: f64 = 1e-15;
    if fs.is_empty() {
        return None;
    }
    let sum: f64 = fs
        .iter()
        .map(|f| {
            let p = f.prob.clamp(EPS, 1.0 - EPS);
            if f.happened { -p.ln() } else { -(1.0 - p).ln() }
        })
        .sum();
    Some(sum / fs.len() as f64)
}

/// Closing-line value in percentage points of implied probability, for a side
/// we priced at `entry_prob` (no-vig) that closed at `closing_prob` (no-vig).
/// Positive = the market moved toward our side after we published.
pub fn clv_pp(entry_prob: f64, closing_prob: f64) -> f64 {
    (closing_prob - entry_prob) * 100.0
}

/// Mean CLV over many picks. `None` if empty.
pub fn mean_clv_pp(pairs: &[(f64, f64)]) -> Option<f64> {
    if pairs.is_empty() {
        return None;
    }
    Some(pairs.iter().map(|(e, c)| clv_pp(*e, *c)).sum::<f64>() / pairs.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(prob: f64, happened: bool) -> Forecast {
        Forecast { prob, happened }
    }
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn brier_perfect_and_worst() {
        assert!(close(brier_score(&[f(1.0, true), f(0.0, false)]).unwrap(), 0.0));
        assert!(close(brier_score(&[f(1.0, false), f(0.0, true)]).unwrap(), 1.0));
    }

    #[test]
    fn brier_coin_flip_is_a_quarter() {
        let fs = [f(0.5, true), f(0.5, false), f(0.5, true), f(0.5, false)];
        assert!(close(brier_score(&fs).unwrap(), 0.25));
    }

    #[test]
    fn brier_known_value() {
        // (0.8-1)^2 = 0.04 ; (0.3-0)^2 = 0.09 ; mean = 0.065
        assert!(close(brier_score(&[f(0.8, true), f(0.3, false)]).unwrap(), 0.065));
    }

    #[test]
    fn well_calibrated_beats_overconfident() {
        // outcomes: 7 of 10 happen. Honest 0.7 vs reckless 0.99
        let outcomes = [true, true, true, true, true, true, true, false, false, false];
        let honest: Vec<_> = outcomes.iter().map(|&h| f(0.7, h)).collect();
        let reckless: Vec<_> = outcomes.iter().map(|&h| f(0.99, h)).collect();
        assert!(brier_score(&honest).unwrap() < brier_score(&reckless).unwrap());
        assert!(log_loss(&honest).unwrap() < log_loss(&reckless).unwrap());
    }

    #[test]
    fn log_loss_coin_flip_is_ln2() {
        let fs = [f(0.5, true), f(0.5, false)];
        assert!(close(log_loss(&fs).unwrap(), std::f64::consts::LN_2));
    }

    #[test]
    fn log_loss_known_value() {
        // -ln(0.8) and -ln(1-0.3)
        let expect = (-(0.8f64).ln() + -(0.7f64).ln()) / 2.0;
        assert!(close(log_loss(&[f(0.8, true), f(0.3, false)]).unwrap(), expect));
    }

    #[test]
    fn log_loss_is_finite_for_certain_wrong_forecast() {
        let v = log_loss(&[f(1.0, false)]).unwrap();
        assert!(v.is_finite() && v > 30.0);
    }

    #[test]
    fn empty_inputs_return_none() {
        assert!(brier_score(&[]).is_none());
        assert!(log_loss(&[]).is_none());
        assert!(mean_clv_pp(&[]).is_none());
    }

    #[test]
    fn clv_positive_when_market_moves_to_our_side() {
        assert!(close(clv_pp(0.50, 0.53), 3.0));
        assert!(clv_pp(0.50, 0.47) < 0.0);
        assert!(close(mean_clv_pp(&[(0.50, 0.53), (0.60, 0.59)]).unwrap(), 1.0));
    }
}
