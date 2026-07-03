//! Tracked link building. PRD P0 #3-4: every pick carries Betchu + >=1
//! affiliate book as tracked links, odds-shopping-agnostic presentation.

use std::env;

pub struct LinkContext<'a> {
    /// "email" | "telegram" — attribution per surface.
    pub surface: &'a str,
    pub tier: &'a str,
    pub game_id: Option<&'a str>,
    /// Per-user attribution where available (telegram user id, beehiiv id).
    pub subscriber_ref: Option<&'a str>,
}

/// Affiliate program URL templates. `{sub}` is replaced with the subId payload.
/// Fill in real program links as affiliate accounts are approved:
///   ("draftkings", "https://dkng.co/oddsports?subid={sub}"),
///   ("fanduel", "https://fanduel.com/aff/oddsports?sub={sub}"),
fn templates() -> Vec<(&'static str, String)> {
    let betchu_base = env::var("BETCHU_REFERRAL_BASE")
        .unwrap_or_else(|_| "https://betchu.example/r/oddsports".into());
    vec![("betchu", format!("{betchu_base}?sub={{sub}}"))]
}

pub fn tracked_link(book: &str, ctx: &LinkContext) -> Option<String> {
    let tmpl = templates()
        .into_iter()
        .find(|(b, _)| b.eq_ignore_ascii_case(book))
        .map(|(_, t)| t)?;
    let sub = [
        ctx.surface,
        ctx.tier,
        ctx.game_id.unwrap_or("-"),
        ctx.subscriber_ref.unwrap_or("-"),
    ]
    .join("_");
    Some(tmpl.replace("{sub}", &urlencode(&sub)))
}

pub fn available_books() -> Vec<&'static str> {
    templates().into_iter().map(|(b, _)| b).collect()
}

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                c.to_string()
            } else {
                c.to_string().bytes().map(|b| format!("%{b:02X}")).collect()
            }
        })
        .collect()
}
