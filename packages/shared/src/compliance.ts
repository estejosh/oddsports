/**
 * Compliance blocks and content lint. PRD Sections 7 & 9.
 * These are P0 constraints: sends are BLOCKED if assertCompliant throws.
 * Wording of RG_DISCLOSURE and RELATED_PARTY_DISCLOSURE is pending legal
 * sign-off (PRD open questions) — treat current text as placeholder that is
 * safe-by-default, not final.
 */

export const RG_DISCLOSURE = [
  "🔞 21+ (or legal betting age in your jurisdiction). Gambling involves risk —",
  "never bet more than you can afford to lose.",
  "If you or someone you know has a gambling problem, call 1-800-GAMBLER (US)",
  "or visit https://www.begambleaware.org.",
].join(" ");

export const RELATED_PARTY_DISCLOSURE =
  "Disclosure: OddSports and Betchu are affiliated companies. " +
  "Links to sportsbooks may earn us a commission.";

export const ANALYSIS_DISCLAIMER =
  "All content is analysis and opinion for entertainment purposes. " +
  "Nothing here is a guarantee of outcome or financial advice.";

/**
 * Banned vocabulary — language implying certainty is a regulatory and
 * reputational hazard. Enforced as a lint step in the pipeline before send.
 */
export const BANNED_PHRASES: RegExp[] = [
  /\block(s)?\b(?!\s*(in|down|ed))/i, // "lock of the day" (not "locked in")
  /\bguaranteed?\b/i,
  /\bcan'?t\s+lose\b/i,
  /\bsure\s+thing\b/i,
  /\bfree\s+money\b/i,
  /\brisk[- ]free\b/i,
  /\b100%\s*(winner|win|hit)\b/i,
];

export interface LintViolation {
  phrase: string;
  index: number;
  excerpt: string;
}

export function lintContent(body: string): LintViolation[] {
  const violations: LintViolation[] = [];
  for (const re of BANNED_PHRASES) {
    const m = re.exec(body);
    if (m) {
      violations.push({
        phrase: m[0],
        index: m.index,
        excerpt: body.slice(Math.max(0, m.index - 30), m.index + m[0].length + 30),
      });
    }
  }
  return violations;
}

/** Template lock: content cannot ship without required blocks. Throws. */
export function assertCompliant(fullIssueBody: string): void {
  const missing: string[] = [];
  if (!fullIssueBody.includes("1-800-GAMBLER")) missing.push("RG disclosure");
  if (!fullIssueBody.includes("affiliated")) missing.push("related-party disclosure");
  if (missing.length > 0) {
    throw new Error(`COMPLIANCE BLOCK — missing required blocks: ${missing.join(", ")}`);
  }
  const violations = lintContent(fullIssueBody);
  if (violations.length > 0) {
    throw new Error(
      `COMPLIANCE BLOCK — banned phrases: ` +
        violations.map((v) => `"${v.phrase}" (…${v.excerpt}…)`).join("; ")
    );
  }
}
