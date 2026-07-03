/**
 * Token budget metering. PRD Section 6 #6: hard daily USD ceiling,
 * alert at 80%, degrade gracefully at 100% (skip lowest-value games,
 * never skip compliance blocks).
 */

// $/MTok — keep in sync with https://docs.anthropic.com/en/docs/about-claude/pricing
const PRICING: Record<string, { input: number; output: number }> = {
  "claude-haiku-4-5-20251001": { input: 1, output: 5 },
  "claude-sonnet-5": { input: 3, output: 15 },
};

export class TokenBudget {
  private spentUsd = 0;
  readonly ceilingUsd: number;
  readonly alertThreshold: number;
  onAlert?: (spent: number, ceiling: number) => void;
  private alerted = false;

  constructor(
    ceilingUsd = Number(process.env.DAILY_TOKEN_BUDGET_USD ?? 10),
    alertThreshold = Number(process.env.BUDGET_ALERT_THRESHOLD ?? 0.8)
  ) {
    this.ceilingUsd = ceilingUsd;
    this.alertThreshold = alertThreshold;
  }

  record(model: string, inputTokens: number, outputTokens: number): number {
    const p = PRICING[model];
    if (!p) throw new Error(`Unknown model pricing: ${model}`);
    const cost = (inputTokens * p.input + outputTokens * p.output) / 1_000_000;
    this.spentUsd += cost;
    if (!this.alerted && this.spentUsd >= this.ceilingUsd * this.alertThreshold) {
      this.alerted = true;
      this.onAlert?.(this.spentUsd, this.ceilingUsd);
    }
    return cost;
  }

  get spent(): number {
    return this.spentUsd;
  }

  get exhausted(): boolean {
    return this.spentUsd >= this.ceilingUsd;
  }

  get remainingUsd(): number {
    return Math.max(0, this.ceilingUsd - this.spentUsd);
  }
}
