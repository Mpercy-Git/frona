import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const client = vi.hoisted(() => ({
  api: { get: vi.fn(), post: vi.fn(), delete: vi.fn() },
  API_URL: "",
}));

vi.mock("@/lib/api-client", () => client);

import { TriggersSection } from "../triggers-section";

const doorbell = {
  id: "tok-1",
  name: "Front doorbell",
  prefix: "eyJhbGciOiJF...",
  expires_at: "2099-01-01T00:00:00Z",
  last_used_at: null,
  created_at: "2026-10-07T09:00:00Z",
};

describe("TriggersSection", () => {
  beforeEach(() => {
    client.api.get.mockReset();
    client.api.post.mockReset();
    client.api.delete.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("lists this agent's trigger tokens", async () => {
    client.api.get.mockResolvedValue([doorbell]);
    render(<TriggersSection agentId="agent-1" />);

    expect(await screen.findByText("Front doorbell")).toBeInTheDocument();
    expect(screen.getByText(/never used/)).toBeInTheDocument();
    expect(client.api.get).toHaveBeenCalledWith("/api/agents/agent-1/trigger-tokens");
  });

  it("shows a new token once, with an example request", async () => {
    client.api.get.mockResolvedValueOnce([]).mockResolvedValueOnce([doorbell]);
    client.api.post.mockResolvedValue({ ...doorbell, token: "secret-token-value" });
    render(<TriggersSection agentId="agent-1" />);
    await screen.findByText("No trigger tokens yet.");

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Front doorbell" } });
    fireEvent.change(screen.getByLabelText("Expires after"), { target: { value: "90" } });
    fireEvent.click(screen.getByRole("button", { name: "Create token" }));

    expect(await screen.findByTestId("minted-token")).toHaveTextContent("secret-token-value");
    expect(client.api.post).toHaveBeenCalledWith("/api/agents/agent-1/trigger-tokens", {
      name: "Front doorbell",
      expires_in_days: 90,
    });
    // The example on screen never repeats the secret.
    expect(screen.getByText(/\/api\/agents\/agent-1\/trigger/)).toHaveTextContent("<token>");

    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.queryByTestId("minted-token")).not.toBeInTheDocument();
  });

  it("revokes a token after confirmation", async () => {
    client.api.get.mockResolvedValue([doorbell]);
    client.api.delete.mockResolvedValue(undefined);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<TriggersSection agentId="agent-1" />);
    await screen.findByText("Front doorbell");

    fireEvent.click(screen.getByRole("button", { name: "Revoke" }));

    await waitFor(() => expect(screen.queryByText("Front doorbell")).not.toBeInTheDocument());
    expect(client.api.delete).toHaveBeenCalledWith("/api/auth/tokens/tok-1");
  });

  it("keeps a token when the revoke is cancelled", async () => {
    client.api.get.mockResolvedValue([doorbell]);
    vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<TriggersSection agentId="agent-1" />);
    await screen.findByText("Front doorbell");

    fireEvent.click(screen.getByRole("button", { name: "Revoke" }));

    expect(client.api.delete).not.toHaveBeenCalled();
    expect(screen.getByText("Front doorbell")).toBeInTheDocument();
  });
});
