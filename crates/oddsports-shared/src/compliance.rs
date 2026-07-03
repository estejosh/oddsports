//! Compliance blocks and content lint. PRD Sections 7 & 9.
//! P0 constraints: sends are BLOCKED if `assert_compliant` errors.
//! Wording pending legal sign-off — current text is safe-by-default placeholder.

use anyhow::{bail, Result};
use regex::Regex;
use std::sync::OnceLock;

pub const RG_DISCLOSURE: &str = "🔞 21+ (or legal betting age in your jurisdiction). Gambling involves risk — \
never bet more than you can afford to lose. If you or someone you know has a gambling problem, \
call 1-800-GAMBLER (US) or visit https://www.begambleaware.org.";

pub const RELATED_PARTY_DISCLOSURE: &str = "Disclosure: OddSports and Betchu are affiliated companies. \
Links to sportsbooks may earn us a commission.";

// NB: worded to pass our own banned-phrase lint (no "guarantee[d]").
pub const ANALYSIS_DISCLAIMER: &str = "All content is analysis and opinion for entertainment purposes. \
No outcome is ever certain; nothing here is financial advice.";

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
            let start = m.start().saturating_sub(30);
            let end = (m.end() + 30).min(body.len());
            violations.push(LintViolation {
                phrase: m.as_str().trim().to_string(),
                excerpt: body[start..end].to_string(),
            });
        }
    }
    violations
}

/// Template lock: content cannot ship without required blocks. Errors abort the send.
pub fn assert_compliant(full_issue_body: &str) -> Result<()> {
    let mut missing = Vec::new();
    if !full_issue_body.contains("1-800-GAMBLER") {
        missing.push("RG disclosure");
    }
    if !full_issue_body.contains("affiliated") {
        missing.push("related-party disclosure");
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

/// Compliance footer appended to every rendered block.
pub fn compliance_footer() -> String {
    format!("\n\n---\n{ANALYSIS_DISCLAIMER}\n{RELATED_PARTY_DISCLOSURE}\n{RG_DISCLOSURE}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
