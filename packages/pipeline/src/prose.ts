/**
 * LLM prose generation with tiered model routing. PRD Section 6 #3:
 * cheap model for Free/Starter, strong model for Analyst/Sharp only.
 * The LLM receives structured factors from the deterministic model and
 * turns them into readable prose — it never invents numbers or picks.
 */
import Anthropic from "@anthropic-ai/sdk";
import { Tier, RG_DISCLOSURE, RELATED_PARTY_DISCLOSURE, ANALYSIS_DISCLAIMER, lintContent } from "@oddsports/shared";
import type { Game, ModelOutput } from "@oddsports/shared";
import type { TokenBudget } from "./budget.js";

const MODEL_CHEAP = process.env.MODEL_CHEAP ?? "claude-haiku-4-5-20251001";
const MODEL_STRONG = process.env.MODEL_STRONG ?? "claude-sonnet-5";

const client = new Anthropic();

// Shared system prompt — identical across all calls for prompt-cache hits (PRD 6 #5).
const SYSTEM_PROMPT = `You are the analysis writer for OddSports, a sports betting newsletter.
You will receive structured model output (pick, edge, confidence, factors). Write prose that
explains it at the requested depth. Rules:
- NEVER invent statistics, numbers, or factors not present in the input.
- NEVER use language implying certainty: no "lock", "guaranteed", "can't lose", "sure thing", "free money", "risk-free".
- Always frame as analysis/opinion, acknowledge variance and risk.
- Match the requested depth exactly: "brief" = 1-2 sentences; "standard" = short paragraph with the key stats; "deep" = full factor-by-factor breakdown with line movement context.`;

export interface ProseResult {
  text: string;
  inputTokens: number;
  outputTokens: number;
}

export async function writePickProse(
  game: Game,
  model: ModelOutput,
  tier: Tier,
  budget: TokenBudget
): Promise<ProseResult> {
  if (budget.exhausted) {
    // Graceful degradation: template fallback, never a skipped compliance block.
    return {
      text: `${model.side} — edge ${model.edgePct} pts vs market consensus. Confidence ${model.confidence}/5.`,
      inputTokens: 0,
      outputTokens: 0,
    };
  }

  const depth = tier >= Tier.Analyst ? "deep" : tier === Tier.Starter ? "standard" : "brief";
  const llmModel = tier >= Tier.Analyst ? MODEL_STRONG : MODEL_CHEAP;

  const res = await client.messages.create({
    model: llmModel,
    max_tokens: depth === "deep" ? 800 : depth === "standard" ? 300 : 100,
    system: [{ type: "text", text: SYSTEM_PROMPT, cache_control: { type: "ephemeral" } }],
    messages: [
      {
        role: "user",
        content: `Depth: ${depth}\nGame: ${game.away} @ ${game.home} (${game.sport}, starts ${game.startsAt})\nModel output:\n${JSON.stringify(model, null, 2)}`,
      },
    ],
  });

  budget.record(llmModel, res.usage.input_tokens, res.usage.output_tokens);

  const text = res.content
    .filter((b): b is Anthropic.TextBlock => b.type === "text")
    .map((b) => b.text)
    .join("\n");

  // Lint the LLM's output too — the system prompt forbids banned phrases but we verify.
  const violations = lintContent(text);
  if (violations.length > 0) {
    throw new Error(`LLM prose failed compliance lint: ${violations.map((v) => v.phrase).join(", ")}`);
  }

  return { text, inputTokens: res.usage.input_tokens, outputTokens: res.usage.output_tokens };
}

/** Compliance footer appended to every rendered block. */
export function complianceFooter(): string {
  return `\n\n---\n${ANALYSIS_DISCLAIMER}\n${RELATED_PARTY_DISCLOSURE}\n${RG_DISCLOSURE}`;
}
