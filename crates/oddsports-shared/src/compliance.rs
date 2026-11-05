//! Compliance blocks and content lint. PRD Sections 7 & 9.
//! P0 constraints: sends are BLOCKED if `assert_compliant` errors.
//! Wording pending legal sign-off — current text is safe-by-default placeholder.

use anyhow::{bail, Result};
use regex::Regex;
use std::sync::OnceLock;

pub const RG_DISCLOSURE: &str = "🔞 21+ (or legal betting age in your jurisdiction). Gambling involves risk — \
never bet more than you can afford to lose. If you or someone you know has a gambling problem, \
call 1-800-GAMBLER (US) or visit https://www.begambleaware.org.";

/// Neutrality statement. OddSports is independent analysis; it names no preferred operator.
pub const NEUTRALITY_DISCLAIMER: &str =
    "OddSports is independent analysis and does not endorse any sportsbook, operator or affiliate program.";

/// Substring `assert_compliant` requires: proves the neutrality disclaimer is present.
pub const NEUTRALITY_MARKER: &str = "does not endorse any sportsbook";

/// Headline analysis disclaimer, required on every issue and bot reply.
pub const ANALYSIS_HEADLINE: &str = "Analysis, not betting advice.";

// NB: worded to pass our own banned-phrase lint (no "guarantee[d]").
pub const ANALYSIS_DISCLAIMER: &str = "Analysis, not betting advice. All content is analysis and opinion \
for entertainment purposes. No outcome is ever certain; nothing here is financial advice.";

/// Language implying certainty is a regulatory and reputational hazard.
/// Enforced as a lint step before every send.
fn banned_phrases() -> &'static [Regex] {
    static RE: OnceLock<Vec<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        [
            r"(?i)\blocks?\b", // "lock of the day" (\b excludes "locked", "lockdown")
            r"(?i)\bguaranteed?\b",
            r"(?i)\bcan'?t\s+lose\b",
            r"(?i)\bsure\s+thing\b",
            r"(?i)\bfree\s+money\b",
            r"(?i)\brisk[- ]free\b",
            r"(?i)\b100%\s*(winner|win|hit)\b",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("valid banned-phrase regex"))
        .collect()
    })
}

#[derive(Debug)]
pub struct LintViolation {
    pub phrase: String,
    pub excerpt: String,
}

pub fn lint_content(body: &str) -> Vec<LintViolation> {
    let mut violations = Vec::new();
    for re in banned_phrases() {
        if let Some(m) = re.find(body) {
            let start = floor_char_boundary(body, m.start().saturating_sub(30));
            let end = ceil_char_boundary(body, (m.end() + 30).min(body.len()));
            violations.push(LintViolation {
                phrase: m.as_str().trim().to_string(),
                excerpt: body[start..end].to_string(),
            });
        }
    }
    violations
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Template lock: content cannot ship without required blocks. Errors abort the send.
pub fn assert_compliant(full_issue_body: &str) -> Result<()> {
    let mut missing = Vec::new();
    if !full_issue_body.contains("1-800-GAMBLER") {
        missing.push("RG disclosure");
    }
    if !full_issue_body.contains(NEUTRALITY_MARKER) {
        missing.push("neutrality disclaimer");
    }
    if !full_issue_body.contains(ANALYSIS_HEADLINE) {
        missing.push("analysis disclaimer");
    }
    if !missing.is_empty() {
        bail!("COMPLIANCE BLOCK — missing required blocks: {}", missing.join(", "));
    }
    let violations = lint_content(full_issue_body);
    if !violations.is_empty() {
        bail!(
            "COMPLIANCE BLOCK — banned phrases: {}",
            violations
                .iter()
                .map(|v| format!("{:?} (…{}…)", v.phrase, v.excerpt))
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    Ok(())
}

/// Compliance footer appended to every rendered block. Not overridable by templates.
pub fn compliance_footer() -> String {
    format!("\n\n---\n{ANALYSIS_DISCLAIMER}\n{NEUTRALITY_DISCLAIMER}\n{RG_DISCLOSURE}")
}

/// Read-time scrub for slates stored before the repo went vendor-neutral.
/// Old bodies carry a "Bet at:" referral row and an operator-affiliation
/// disclosure. The stored record is left untouched (it is immutable by design);
/// anything read back for display passes through here:
///   * "Bet at: ..." lines are dropped;
///   * affiliation / commission disclosure lines become the neutrality disclaimer.
pub fn scrub_legacy_promo(body: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in body.lines() {
        let l = line.trim_start().to_lowercase();
        if l.starts_with("bet at:") {
            continue;
        }
        if l.contains("affiliated compan") || l.contains("may earn us a commission") {
            out.push(NEUTRALITY_DISCLAIMER.to_string());
            continue;
        }
        out.push(line.to_string());
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_removes_legacy_referral_row_and_affiliation_line() {
        let old = "**A @ B** — B -3\n\nprose\n\nBet at: [x](https://ref.example/r?sub=1)\n\n---\nAll content is analysis.\nDisclosure: OddSports and X are affiliated companies. Links to sportsbooks may earn us a commission.\n🔞 21+ ... 1-800-GAMBLER";
        let s = scrub_legacy_promo(old);
        assert!(!s.contains("Bet at"));
        assert!(!s.contains("ref.example"));
        assert!(!s.to_lowercase().contains("affiliated"));
        assert!(!s.to_lowercase().contains("commission"));
        assert!(s.contains(NEUTRALITY_DISCLAIMER));
        assert!(s.contains("prose") && s.contains("1-800-GAMBLER"));
    }

    #[test]
    fn scrub_leaves_clean_bodies_unchanged() {
        let clean = format!("pick{}", compliance_footer());
        assert_eq!(scrub_legacy_promo(&clean), clean);
    }

    #[test]
    fn blocks_missing_disclosures() {
        assert!(assert_compliant("just a pick, no disclosures").is_err());
    }

    #[test]
    fn blocks_banned_phrases() {
        let body = format!("This is a guaranteed winner!{}", compliance_footer());
        assert!(assert_compliant(&body).is_err());
    }

    #[test]
    fn passes_clean_content() {
        let body = format!("Chiefs -3.5 looks strong per the model.{}", compliance_footer());
        assert!(assert_compliant(&body).is_ok());
    }

    #[test]
    fn blocks_when_only_rg_present() {
        let body = format!("pick\n{RG_DISCLOSURE}");
        let err = assert_compliant(&body).unwrap_err().to_string();
        assert!(err.contains("neutrality disclaimer"));
        assert!(err.contains("analysis disclaimer"));
    }

    #[test]
    fn footer_carries_no_operator_or_affiliate_promotion() {
        let f = compliance_footer().to_lowercase();
        assert!(!f.contains("commission"));
        assert!(!f.contains("sign up"));
        assert!(!f.contains("deposit"));
        assert!(!f.contains("referral"));
    }

    #[test]
    fn lint_excerpt_is_utf8_safe() {
        // multibyte chars right next to the match must not panic the slicer
        let body = "🔞🔞🔞🔞🔞🔞🔞🔞 guaranteed 🔞🔞🔞🔞🔞🔞🔞🔞";
        assert_eq!(lint_content(body).len(), 1);
    }
}
