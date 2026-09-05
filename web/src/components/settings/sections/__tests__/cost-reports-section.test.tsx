import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import type { AdminSpendAnalysis, CostReport } from "@/lib/types";

const getAdminUsage = vi.fn();
const listCostReports = vi.fn();
const runCostAnalysis = vi.fn();

// `vi.mock` is hoisted above every import, so the stand-in error class has to
// be declared inside the factory rather than referenced from module scope.
vi.mock("@/lib/api-client", () => {
  class ApiError extends Error {
    status: number;
    constructor(status: number, message: string) {
      super(message);
      this.status = status;
    }
  }
  return {
    ApiError,
    getAdminUsage: (...a: unknown[]) => getAdminUsage(...a),
    listCostReports: (...a: unknown[]) => listCostReports(...a),
    runCostAnalysis: (...a: unknown[]) => runCostAnalysis(...a),
  };
});

import { ApiError } from "@/lib/api-client";
import { CostReportsSection } from "../cost-reports-section";

function analysis(overrides: Partial<AdminSpendAnalysis> = {}): AdminSpendAnalysis {
  return {
    window_since: "2026-08-01T00:00:00Z",
    window_until: "2026-08-31T00:00:00Z",
    window_days: 30,
    totals: {
      input_tokens: 1_000_000,
      cached_input_tokens: 200_000,
      output_tokens: 100_000,
      cost_usd: 42,
      calls: 500,
    },
    metered_cost_usd: 22,
    subscription_cost_usd: 20,
    subscription_list_value_usd: 18,
    self_hosted_list_value_usd: 2,
    uncosted_calls: 0,
    providers: [],
    models: [],
    by_model_group: {},
    by_kind: {},
    top_users: [],
    pricing_version: "abc123",
    ...overrides,
  };
}

function report(overrides: Partial<CostReport> = {}): CostReport {
  return {
    id: "r1",
    user_id: "u1",
    window_since: "2026-08-01T00:00:00Z",
    window_until: "2026-08-31T00:00:00Z",
    summary: "Spend is concentrated in the reasoning group.",
    recommendations: [],
    totals: {
      input_tokens: 0,
      cached_input_tokens: 0,
      output_tokens: 0,
      cost_usd: 0,
      calls: 0,
    },
    metered_cost_usd: 22,
    subscription_cost_usd: 20,
    subscription_list_value_usd: 18,
    estimated_monthly_savings_usd: null,
    pricing_version: "abc123",
    created_at: "2026-09-01T09:00:00Z",
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  getAdminUsage.mockResolvedValue(analysis());
  listCostReports.mockResolvedValue([]);
  runCostAnalysis.mockResolvedValue({ task: { id: "t1" } });
});

describe("CostReportsSection", () => {
  it("separates money spent from value covered by a subscription", async () => {
    render(<CostReportsSection />);

    // The whole point of the billing model: these are different numbers and
    // must not be presented as one total.
    await waitFor(() => expect(screen.getByText("$22.00")).toBeInTheDocument());
    expect(screen.getByText("$20.00")).toBeInTheDocument();
    expect(screen.getByText("$18.00")).toBeInTheDocument();
    expect(screen.getByText(/Metered spend/i)).toBeInTheDocument();
    expect(screen.getByText(/Covered by subscriptions/i)).toBeInTheDocument();
  });

  it("warns that uncosted calls understate the totals", async () => {
    getAdminUsage.mockResolvedValue(analysis({ uncosted_calls: 1500 }));
    render(<CostReportsSection />);

    await waitFor(() =>
      expect(screen.getByText(/unmeasured rather than zero/i)).toBeInTheDocument(),
    );
  });

  it("says so when no reports exist yet", async () => {
    render(<CostReportsSection />);
    await waitFor(() =>
      expect(screen.getByText(/No cost reports yet/i)).toBeInTheDocument(),
    );
  });

  it("renders a recommendation with a saving shown as a reduction", async () => {
    listCostReports.mockResolvedValue([
      report({
        estimated_monthly_savings_usd: -12.5,
        recommendations: [
          {
            kind: "switch_model",
            model_group: "reasoning",
            from: "openai/gpt-5",
            to: "anthropic/claude-sonnet-5",
            rationale: "Same capability set at a lower blended rate.",
            estimated_monthly_delta_usd: -12.5,
            confidence: "high",
          },
        ],
      }),
    ]);
    render(<CostReportsSection />);

    await waitFor(() => expect(screen.getByText("Switch model")).toBeInTheDocument());
    expect(screen.getByText("reasoning")).toBeInTheDocument();
    expect(screen.getByText("openai/gpt-5 → anthropic/claude-sonnet-5")).toBeInTheDocument();
    // Negative deltas read as savings, with a minus sign rather than "-$-12.5".
    expect(screen.getAllByText("−$12.50/mo").length).toBeGreaterThan(0);
  });

  it("reports an empty recommendation list as a finding, not a blank", async () => {
    listCostReports.mockResolvedValue([report()]);
    render(<CostReportsSection />);

    await waitFor(() =>
      expect(screen.getByText("No changes recommended.")).toBeInTheDocument(),
    );
  });

  it("explains a 403 rather than showing a generic failure", async () => {
    getAdminUsage.mockRejectedValue(new ApiError(403, "Not permitted"));
    render(<CostReportsSection />);

    await waitFor(() =>
      expect(
        screen.getByText(/do not have permission to view server-wide spend/i),
      ).toBeInTheDocument(),
    );
  });

  it("queues an analysis and says it runs in the background", async () => {
    render(<CostReportsSection />);
    await waitFor(() => expect(getAdminUsage).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: /Analyse now/i }));

    await waitFor(() => expect(runCostAnalysis).toHaveBeenCalledTimes(1));
    expect(await screen.findByText(/Analysis queued/i)).toBeInTheDocument();
  });

  it("shows allowance consumption for a subscription provider", async () => {
    getAdminUsage.mockResolvedValue(
      analysis({
        providers: [
          {
            provider: "anthropic",
            billing: {
              kind: "subscription",
              monthly_cost: 20,
              currency: "GBP",
              overage_is_metered: true,
              included_spend_usd: 20,
            },
            recorded_kinds: ["subscription"],
            rollup: {
              input_tokens: 100,
              cached_input_tokens: 0,
              output_tokens: 50,
              cost_usd: 24,
              calls: 10,
            },
            prorated_fee_usd: 20,
            allowance: {
              included_tokens: null,
              tokens_used: 150,
              included_spend_usd: 20,
              list_value_used_usd: 24,
              used_pct: 120,
              exceeded: true,
              overage_is_metered: true,
            },
          },
        ],
      }),
    );
    render(<CostReportsSection />);

    await waitFor(() => expect(screen.getByText("anthropic")).toBeInTheDocument());
    expect(screen.getByText("Subscription")).toBeInTheDocument();
    // The fee is quoted in its configured currency, never converted.
    expect(screen.getByText(/fee 20 GBP\/mo/)).toBeInTheDocument();
    expect(screen.getByText(/120% of allowance — billing overage/)).toBeInTheDocument();
  });
});
