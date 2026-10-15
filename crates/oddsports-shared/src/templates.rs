//! Output templates with an operator-supplied override directory.
//!
//! The public repo ships ONLY neutral built-in templates: no calls to action,
//! no referral codes, no operator names. A private deployment can restyle or
//! extend output by pointing `TEMPLATE_DIR` at a directory containing any of:
//!
//!   pick.md          one pick block (placeholders below)
//!   bot_start.txt    the Telegram /start message
//!
//! A missing file falls back to the built-in. The compliance footer is NOT a
//! template: it is always appended by code and `assert_compliant` still runs on
//! the result, so an override cannot drop required disclosures.
//!
//! `pick.md` placeholders: {matchup} {side} {stars} {fair_odds} {prose} {sizing}
//! Unknown placeholders are left as-is. `bot_start.txt` has no placeholders.

use std::path::PathBuf;

pub const DEFAULT_PICK: &str =
    "**{matchup}** — {side}\n\nConfidence: {stars}\n\n{fair_odds}{prose}{sizing}";

pub const DEFAULT_BOT_START: &str = "🏟 *OddSports* — independent sports betting analysis with fair-odds estimation.\n\n\
Free: top daily picks with confidence ratings. Paid tiers unlock the full slate, model breakdowns, \
line movement, unit sizing, and live alerts.\n\n\
Link your newsletter account: /link <email>\n\n\
Commands: /today /record /link /help /rg";

#[derive(Debug, Clone, Default)]
pub struct Templates {
    dir: Option<PathBuf>,
    attribution: Option<String>,
    vendors: crate::vendors::Vendors,
}

impl Templates {
    /// Built-ins only.
    pub fn builtin() -> Self {
        Self::default()
    }

    /// Reads `TEMPLATE_DIR` (unset/empty = built-ins only) and `DATA_ATTRIBUTION`
    /// (a neutral data-source credit required by some feeds' terms, e.g.
    /// "Market data via The Odds API (https://the-odds-api.com)").
    pub fn from_env() -> Self {
        let get = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let dir = get("TEMPLATE_DIR").map(PathBuf::from);
        // Vendors: VENDORS_FILE, else vendors.json inside TEMPLATE_DIR. A bad
        // file fails CLOSED (no vendors) with a warning, never half-loads.
        let vendors_path = get("VENDORS_FILE")
            .map(PathBuf::from)
            .or_else(|| dir.as_ref().map(|d| d.join("vendors.json")).filter(|p| p.exists()));
        let vendors = match vendors_path {
            Some(p) => crate::vendors::Vendors::from_file(&p).unwrap_or_else(|e| {
                eprintln!("WARNING: vendors not loaded ({e}); continuing with no vendors");
                crate::vendors::Vendors::default()
            }),
            None => crate::vendors::Vendors::default(),
        };
        Self { dir, attribution: get("DATA_ATTRIBUTION"), vendors }
    }

    pub fn with_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: Some(dir.into()), ..Self::default() }
    }

    pub fn with_vendors(mut self, vendors: crate::vendors::Vendors) -> Self {
        self.vendors = vendors;
        self
    }

    /// Configured vendors (empty unless a deployment supplies them).
    pub fn vendors(&self) -> &crate::vendors::Vendors {
        &self.vendors
    }

    pub fn with_attribution(mut self, text: impl Into<String>) -> Self {
        self.attribution = Some(text.into());
        self
    }

    /// Data-source credit appended to every pick, if configured. Always added by
    /// code, so a template override cannot drop it.
    pub fn attribution(&self) -> Option<&str> {
        self.attribution.as_deref()
    }

    fn load(&self, file: &str, default: &str) -> String {
        if let Some(dir) = &self.dir {
            if let Ok(s) = std::fs::read_to_string(dir.join(file)) {
                if !s.trim().is_empty() {
                    return s;
                }
            }
        }
        default.to_string()
    }

    pub fn pick(&self) -> String {
        self.load("pick.md", DEFAULT_PICK)
    }

    pub fn bot_start(&self) -> String {
        self.load("bot_start.txt", DEFAULT_BOT_START)
    }
}

/// Replace `{key}` placeholders. Single pass per key; values are not re-scanned
/// for placeholders by later keys because replacement order is fixed and values
/// are inserted last-to-first-safe via a placeholder-token scan.
pub fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 64);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                match vars.iter().find(|(k, _)| *k == key) {
                    Some((_, v)) => out.push_str(v),
                    None => {
                        out.push('{');
                        out.push_str(key);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oddsports-tpl-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn builtin_has_no_cta_or_operator() {
        let all = format!("{}{}", DEFAULT_PICK, DEFAULT_BOT_START).to_lowercase();
        for bad in ["bet at", "sign up", "deposit", "referral", "affiliate", "bonus", "http"] {
            assert!(!all.contains(bad), "built-in template contains {bad:?}");
        }
    }

    #[test]
    fn missing_dir_or_file_falls_back_to_builtin() {
        assert_eq!(Templates::builtin().pick(), DEFAULT_PICK);
        let d = tmp("empty");
        assert_eq!(Templates::with_dir(&d).pick(), DEFAULT_PICK);
        assert_eq!(Templates::with_dir(d.join("nope")).bot_start(), DEFAULT_BOT_START);
    }

    #[test]
    fn override_file_replaces_builtin() {
        let d = tmp("override");
        std::fs::write(d.join("pick.md"), "PRIVATE {side} / {matchup}").unwrap();
        std::fs::write(d.join("bot_start.txt"), "hello from private deployment").unwrap();
        let t = Templates::with_dir(&d);
        assert_eq!(t.pick(), "PRIVATE {side} / {matchup}");
        assert_eq!(t.bot_start(), "hello from private deployment");
    }

    #[test]
    fn blank_override_is_ignored() {
        let d = tmp("blank");
        std::fs::write(d.join("pick.md"), "   \n").unwrap();
        assert_eq!(Templates::with_dir(&d).pick(), DEFAULT_PICK);
    }

    #[test]
    fn fill_replaces_known_and_keeps_unknown() {
        let out = fill("{a} and {b} and {c}", &[("a", "1"), ("b", "2")]);
        assert_eq!(out, "1 and 2 and {c}");
    }

    #[test]
    fn fill_does_not_expand_placeholders_inside_values() {
        let out = fill("{a}{b}", &[("a", "{b}"), ("b", "X")]);
        assert_eq!(out, "{b}X");
    }

    #[test]
    fn fill_handles_unbalanced_braces() {
        assert_eq!(fill("oops { no close", &[]), "oops { no close");
    }
}
