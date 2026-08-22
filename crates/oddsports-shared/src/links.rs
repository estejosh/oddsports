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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> LinkContext<'static> {
        LinkContext { surface: "email", tier: "sharp", game_id: Some("g1"), subscriber_ref: None }
    }

    #[test]
    fn unknown_book_yields_no_link() {
        assert!(tracked_link("draftkings", &ctx()).is_none());
        assert!(tracked_link("", &ctx()).is_none());
    }

    #[test]
    fn betchu_link_carries_urlencoded_sub_payload() {
        let url = tracked_link("Betchu", &ctx()).expect("betchu is always available");
        assert!(url.contains("?sub="));
        let sub = url.split("sub=").nth(1).unwrap();
        // No raw separators that would break the query string.
        assert!(!sub.contains('&') && !sub.contains('?'));
        // Round-trips to the plain payload.
        let decoded = sub.replace("%40", "@");
        assert!(decoded.contains("email_sharp_g1"));
    }

    #[test]
    fn sub_id_encodes_specials_and_keeps_unreserved() {
        assert_eq!(urlencode("a_b-c.d~e"), "a_b-c.d~e");
        assert_eq!(urlencode("a b/c"), "a%20b%2Fc");
        assert_eq!(urlencode("é"), "%C3%A9"); // multi-byte, per-byte percent-encoding
    }

    #[test]
    fn available_books_includes_betchu_case_insensitively_matched() {
        assert!(available_books().iter().any(|b| b.eq_ignore_ascii_case("BETCHU")));
    }
}
