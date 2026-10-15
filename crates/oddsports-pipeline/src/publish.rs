//! Stage 6 — publish. Renders a pick from a template, appends the
//! non-overridable compliance footer, and enforces two hard gates before
//! anything ships:
//!   * `assert_no_source_odds` — no individual book is named anywhere
//!     (we publish OUR fair odds and the consensus, never a per-source price);
//!   * `assert_compliant` — required disclosures present, no banned phrasing.
//!
//! There is no call to action, referral code or operator link here. Anything
//! like that belongs in a private deployment's template override
//! (`TEMPLATE_DIR`, see `oddsports_shared::templates`).

use crate::real_odds_score::RealOddsScore;
use anyhow::{bail, Result};
use oddsports_shared::{assert_compliant, compliance_footer, templates::fill, Game, ModelOutput, Templates, Tier};
use std::collections::BTreeSet;

fn stars(n: u8) -> String {
    let n = n.min(5) as usize;
    "★".repeat(n) + &"☆".repeat(5 - n)
}

fn fmt_american(o: i32) -> String {
    if o > 0 { format!("+{o}") } else { format!("{o}") }
}

/// "Fair moneyline odds (no-vig consensus, 6 books): Celtics -240 · Knicks +205"
pub fn fair_odds_line(game: &Game, fair: Option<&RealOddsScore>) -> String {
    let Some(f) = fair else { return String::new() };
    let Some((h, a)) = f.fair_american() else { return String::new() };
    let draw = f
        .draw_prob
        .and_then(crate::aggregate::prob_to_american)
        .map(|d| format!(" · Draw {}", fmt_american(d)))
        .unwrap_or_default();
    let adj = if f.factors_applied > 0 { ", adjusted for sourced factors" } else { "" };
    format!(
        "Fair moneyline odds (no-vig consensus of {} books{adj}): {} {} · {} {}{draw} — market agreement {}\n\n",
        f.base.books,
        game.home,
        fmt_american(h),
        game.away,
        fmt_american(a),
        stars(f.base.confidence),
    )
}

/// Every book identifier present in the game's raw lines.
fn book_keys(game: &Game) -> BTreeSet<String> {
    game.spread_lines
        .iter()
        .chain(&game.moneyline_lines)
        .chain(&game.total_lines)
        .map(|l| l.book.trim().to_lowercase())
        .filter(|b| !b.is_empty())
        .collect()
}

fn contains_word(hay: &str, needle: &str) -> bool {
    if needle.len() < 3 {
        return false;
    }
    let mut from = 0;
    while let Some(i) = hay[from..].find(needle) {
        let s = from + i;
        let e = s + needle.len();
        let before_ok = hay[..s].chars().next_back().map_or(true, |c| !c.is_alphanumeric());
        let after_ok = hay[e..].chars().next().map_or(true, |c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        from = s + needle.chars().next().map_or(1, |c| c.len_utf8());
    }
    false
}

/// Hard gate: published text must not name any book that quoted this game.
pub fn assert_no_source_odds(body: &str, game: &Game) -> Result<()> {
    let lower = body.to_lowercase();
    for key in book_keys(game) {
        let spaced = key.replace('_', " ");
        let joined = key.replace('_', "");
        // "williamhill_us" is also published as "williamhill" / "william hill"
        let stem = key.split('_').next().unwrap_or(&key).to_string();
        let mut needles: Vec<String> = vec![key.clone(), spaced, joined, stem.clone()];
        // "williamhill" is also written "william hill", "draftkings" as "draft kings", ...
        let chars: Vec<char> = stem.chars().collect();
        for i in 2..chars.len().saturating_sub(1) {
            let mut s: String = chars[..i].iter().collect();
            s.push(' ');
            s.extend(chars[i..].iter());
            needles.push(s);
        }
        if needles.iter().any(|n| contains_word(&lower, n)) {
            bail!("SOURCE LEAK BLOCK — rendered output names book {:?}", key);
        }
    }
    Ok(())
}

/// Render one pick for one tier and run both gates.
pub fn render_pick(
    templates: &Templates,
    game: &Game,
    mo: &ModelOutput,
    tier: Tier,
    prose: &str,
    fair: Option<&RealOddsScore>,
) -> Result<String> {
    let sizing = if tier >= Tier::Analyst {
        format!("\n\nSuggested size: {}u", mo.suggested_units)
    } else {
        String::new()
    };
    let matchup = format!("{} @ {}", game.away, game.home);
    let stars_s = stars(mo.confidence);
    let fair_s = fair_odds_line(game, fair);
    let body = fill(
        &templates.pick(),
        &[
            ("matchup", &matchup),
            ("side", &mo.side),
            ("stars", &stars_s),
            ("fair_odds", &fair_s),
            ("prose", prose),
            ("sizing", &sizing),
        ],
    );
    let credit = templates.attribution().map(|a| format!("\n\nData: {a}")).unwrap_or_default();
    let core = format!("{body}{credit}");
    // The leak gate checks everything we wrote about the game. The vendor
    // block is exempt: it is a plain alphabetical list, never next to odds.
    assert_no_source_odds(&format!("{core}{}", compliance_footer()), game)?;
    // Vendors (if a deployment configured any) are added by code, with their
    // disclosure, so a template override cannot drop or reorder them.
    let vendor_block = templates.vendors().block("email", tier.name(), &game.id);
    let body = format!("{core}{vendor_block}{}", compliance_footer());
    assert_compliant(&body)?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::testutil::*;
    use crate::{odds_score::score_game, real_odds_score};
    use oddsports_shared::{FactorDirection, MarketType, PickTeam};

    fn mo() -> ModelOutput {
        ModelOutput {
            game_id: "g1".into(),
            market: MarketType::Spread,
            side: "Celtics -4.5".into(),
            pick_team: PickTeam::Home,
            picked_line: -4.5,
            fair_line: -6.0,
            edge_pct: 1.5,
            confidence: 3,
            suggested_units: 1.0,
            factors: vec![],
            line_history: vec![],
        }
    }

    /// Distinctive raw prices that cannot coincide with our fair odds.
    fn market() -> Game {
        two_way(&[
            ("fanduel", -137, 117),
            ("draftkings", -141, 121),
            ("williamhill_us", -139, 119),
            ("betmgm", -138, 118),
            ("pointsbetus", -140, 120),
        ])
    }

    fn fair(g: &Game) -> RealOddsScore {
        real_odds_score::apply(&score_game(g).unwrap(), &[])
    }

    #[test]
    fn raw_per_source_odds_and_book_names_never_appear_in_output() {
        let g = market();
        let body = render_pick(&Templates::builtin(), &g, &mo(), Tier::Analyst, "Solid spot.", Some(&fair(&g))).unwrap();
        let lower = body.to_lowercase();
        for book in ["fanduel", "draftkings", "williamhill", "william hill", "betmgm", "pointsbet"] {
            assert!(!lower.contains(book), "leaked {book}");
        }
        // none of the raw quoted prices appear
        for raw in ["-137", "-141", "-139", "-138", "-140", "+117", "+121", "+119", "+118", "+120"] {
            assert!(!body.contains(raw), "raw price {raw} leaked into: {body}");
        }
        // but our fair odds and consensus count do
        assert!(body.contains("Fair moneyline odds"));
        assert!(body.contains("5 books"));
    }

    #[test]
    fn leak_gate_blocks_prose_that_names_a_book() {
        let g = market();
        let err = render_pick(&Templates::builtin(), &g, &mo(), Tier::Free, "FanDuel has the best number.", Some(&fair(&g)))
            .unwrap_err()
            .to_string();
        assert!(err.contains("SOURCE LEAK"), "{err}");
    }

    #[test]
    fn leak_gate_catches_spaced_and_underscore_variants() {
        let g = market();
        for prose in ["William Hill is high", "williamhill_us is high", "williamhill is high", "DRAFTKINGS shows it"] {
            assert!(assert_no_source_odds(prose, &g).is_err(), "should block: {prose}");
        }
    }

    #[test]
    fn leak_gate_does_not_false_positive_on_substrings() {
        let g = market();
        assert!(assert_no_source_odds("The betmgmt desk and fanduelity are not books", &g).is_ok());
        assert!(assert_no_source_odds("Celtics -4.5, solid spot", &g).is_ok());
    }

    #[test]
    fn every_render_has_required_disclosures() {
        let g = market();
        for tier in [Tier::Free, Tier::Starter, Tier::Analyst, Tier::Sharp] {
            let body = render_pick(&Templates::builtin(), &g, &mo(), tier, "ok", None).unwrap();
            assert!(body.contains("Analysis, not betting advice."));
            assert!(body.contains("does not endorse any sportsbook"));
            assert!(body.contains("1-800-GAMBLER"));
        }
    }

    #[test]
    fn default_render_has_no_cta_or_referral() {
        let g = market();
        let body = render_pick(&Templates::builtin(), &g, &mo(), Tier::Analyst, "ok", Some(&fair(&g))).unwrap().to_lowercase();
        for bad in ["bet at", "sign up", "referral", "promo", "bonus", "http://", "commission"] {
            assert!(!body.contains(bad), "default output contains {bad:?}");
        }
        // the only link allowed is the RG helpline
        assert_eq!(body.matches("https://").count(), 1);
    }

    #[test]
    fn data_attribution_is_appended_by_code_and_survives_template_override() {
        let d = std::env::temp_dir().join(format!("oddsports-pub3-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("pick.md"), "{side}").unwrap();
        let g = market();
        let t = Templates::with_dir(&d).with_attribution("Market data via The Odds API (https://the-odds-api.com)");
        let body = render_pick(&t, &g, &mo(), Tier::Free, "ok", None).unwrap();
        assert!(body.contains("Data: Market data via The Odds API"));
        let none = render_pick(&Templates::builtin(), &g, &mo(), Tier::Free, "ok", None).unwrap();
        assert!(!none.contains("Data:"));
    }

    fn vendors() -> oddsports_shared::Vendors {
        // note: "fanduel" is ALSO a book that quoted this game in `market()`
        oddsports_shared::Vendors::from_json(
            r#"[{"id":"zeta","name":"Zeta Book","url":"https://z.example/go?sub={sub}","commission":true},
                {"id":"fanduel","name":"FanDuel","url":"https://f.example/go?sub={sub}"}]"#,
        )
        .unwrap()
    }

    #[test]
    fn vendors_are_listed_alphabetically_with_disclosure_and_exempt_from_leak_gate() {
        let g = market();
        let t = Templates::builtin().with_vendors(vendors());
        let body = render_pick(&t, &g, &mo(), Tier::Analyst, "ok", Some(&fair(&g))).unwrap();
        assert!(body.contains("Sportsbooks: [FanDuel]"));
        assert!(body.find("FanDuel").unwrap() < body.find("Zeta Book").unwrap());
        assert!(body.contains("may earn OddSports a commission"));
        assert!(!body.to_lowercase().contains("affiliated"));
        assert!(body.contains("sub=email_Analyst_g1"));
        // vendor block is not odds: no raw prices leak next to it
        for raw in ["-137", "-141", "+117", "+121"] {
            assert!(!body.contains(raw));
        }
    }

    #[test]
    fn prose_naming_a_book_is_still_blocked_even_when_that_book_is_a_vendor() {
        let g = market();
        let t = Templates::builtin().with_vendors(vendors());
        let err = render_pick(&t, &g, &mo(), Tier::Free, "FanDuel has the best number.", None).unwrap_err();
        assert!(err.to_string().contains("SOURCE LEAK"));
    }

    #[test]
    fn vendor_block_survives_template_override_and_is_absent_when_unconfigured() {
        let d = std::env::temp_dir().join(format!("oddsports-pub4-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("pick.md"), "{side}").unwrap();
        let g = market();
        let with = render_pick(&Templates::with_dir(&d).with_vendors(vendors()), &g, &mo(), Tier::Free, "ok", None).unwrap();
        assert!(with.contains("Sportsbooks:") && with.contains("no endorsement"));
        let without = render_pick(&Templates::with_dir(&d), &g, &mo(), Tier::Free, "ok", None).unwrap();
        assert!(!without.contains("Sportsbooks:"));
    }

    #[test]
    fn vendor_free_output_is_byte_identical_to_before() {
        let g = market();
        let a = render_pick(&Templates::builtin(), &g, &mo(), Tier::Starter, "ok", None).unwrap();
        let b = render_pick(&Templates::builtin().with_vendors(Default::default()), &g, &mo(), Tier::Starter, "ok", None).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn sizing_only_for_analyst_and_up() {
        let g = market();
        let free = render_pick(&Templates::builtin(), &g, &mo(), Tier::Starter, "ok", None).unwrap();
        let analyst = render_pick(&Templates::builtin(), &g, &mo(), Tier::Analyst, "ok", None).unwrap();
        assert!(!free.contains("Suggested size"));
        assert!(analyst.contains("Suggested size: 1u"));
    }

    #[test]
    fn template_override_is_used_but_footer_cannot_be_dropped() {
        let d = std::env::temp_dir().join(format!("oddsports-pub-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("pick.md"), "PRIVATE LAYOUT :: {side} :: {matchup}").unwrap();
        let g = market();
        let body = render_pick(&Templates::with_dir(&d), &g, &mo(), Tier::Free, "ok", None).unwrap();
        assert!(body.starts_with("PRIVATE LAYOUT :: Celtics -4.5 :: Knicks @ Celtics"));
        assert!(body.contains("does not endorse any sportsbook"));
        assert!(body.contains("1-800-GAMBLER"));
    }

    #[test]
    fn override_cannot_smuggle_a_banned_phrase_past_the_gate() {
        let d = std::env::temp_dir().join(format!("oddsports-pub2-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("pick.md"), "{side} is a guaranteed winner").unwrap();
        let g = market();
        assert!(render_pick(&Templates::with_dir(&d), &g, &mo(), Tier::Free, "ok", None).is_err());
    }

    #[test]
    fn fair_odds_line_notes_adjustment_only_when_factors_applied() {
        let g = market();
        let base = score_game(&g).unwrap();
        let plain = fair_odds_line(&g, Some(&real_odds_score::apply(&base, &[])));
        assert!(!plain.contains("adjusted"));
        let f = oddsports_shared::Factor {
            name: "n".into(),
            direction: FactorDirection::For,
            weight: 1.0,
            detail: "d".into(),
            source: "https://example.org/x".into(),
            as_of: "2026-10-02T09:00:00Z".into(),
        };
        let adj = fair_odds_line(&g, Some(&real_odds_score::apply(&base, &[f])));
        assert!(adj.contains("adjusted for sourced factors"));
    }

    #[test]
    fn no_consensus_means_no_fair_odds_line_but_pick_still_renders() {
        let g = two_way(&[("a", -110, -110)]);
        assert!(score_game(&g).is_none());
        assert_eq!(fair_odds_line(&g, None), "");
        assert!(render_pick(&Templates::builtin(), &g, &mo(), Tier::Free, "ok", None).is_ok());
    }
}
