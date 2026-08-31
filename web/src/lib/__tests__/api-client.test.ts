import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

type Handler = (url: string, init?: RequestInit) => Response | Promise<Response>;

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

/// Fresh module instance per test — the access token and the in-flight refresh
/// are module state.
async function loadClient() {
  vi.resetModules();
  return import("../api-client");
}

let handler: Handler;
let calls: string[];

beforeEach(() => {
  calls = [];
  handler = () => json({});
  vi.stubGlobal("fetch", vi.fn((url: string, init?: RequestInit) => {
    calls.push(`${init?.method ?? "GET"} ${url}`);
    return Promise.resolve(handler(url, init));
  }));
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const refreshCalls = () => calls.filter((c) => c.endsWith("/api/auth/refresh")).length;

describe("api-client: expired access tokens", () => {
  it("refreshes and retries instead of surfacing the expiry", async () => {
    const client = await loadClient();
    client.setAccessToken("stale");

    handler = (url, init) => {
      if (url.endsWith("/api/auth/refresh")) return json({ token: "fresh" });
      const auth = (init?.headers as Record<string, string>)?.["Authorization"];
      if (auth === "Bearer stale") {
        return json({ error: "Session expired", code: "token_expired" }, 401);
      }
      return json({ ok: true });
    };

    await expect(client.api.get("/api/thing")).resolves.toEqual({ ok: true });
    expect(refreshCalls()).toBe(1);
  });

  it("refreshes once for concurrent 401s", async () => {
    const client = await loadClient();
    client.setAccessToken("stale");

    // The server rotates the refresh pair and rejects a replay, so a second
    // concurrent refresh would come back "session gone" — the bug that showed
    // an expiry to the user mid-session.
    handler = (url, init) => {
      if (url.endsWith("/api/auth/refresh")) {
        return refreshCalls() > 1
          ? json({ error: "Refresh token already used or expired" }, 401)
          : json({ token: "fresh" });
      }
      const auth = (init?.headers as Record<string, string>)?.["Authorization"];
      if (auth === "Bearer stale") {
        return json({ error: "Session expired", code: "token_expired" }, 401);
      }
      return json({ ok: true });
    };

    const results = await Promise.all([
      client.api.get("/api/a"),
      client.api.get("/api/b"),
      client.api.get("/api/c"),
    ]);

    expect(results).toEqual([{ ok: true }, { ok: true }, { ok: true }]);
    expect(refreshCalls()).toBe(1);
  });

  it("reports a dead session in plain language, not the server's prose", async () => {
    const client = await loadClient();
    client.setAccessToken("stale");

    handler = (url) =>
      url.endsWith("/api/auth/refresh")
        ? json({ error: "Session expired", code: "token_expired" }, 401)
        : json({ error: "Session expired", code: "token_expired" }, 401);

    const err = await client.api.get("/api/thing").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(client.ApiError);
    const apiErr = err as InstanceType<typeof client.ApiError>;
    expect(apiErr.status).toBe(401);
    expect(apiErr.code).toBe("token_expired");
    expect(apiErr.message).toBe("Your session has expired. Please sign in again.");
  });

  it("announces the dead session once and drops the token", async () => {
    const client = await loadClient();
    client.setAccessToken("stale");

    const expired = vi.fn();
    client.onSessionExpired(expired);

    handler = () => json({ error: "Session expired", code: "token_expired" }, 401);

    await client.api.get("/api/thing").catch(() => {});

    expect(expired).toHaveBeenCalledTimes(1);
    expect(client.getAccessToken()).toBeNull();
  });

  it("keeps the session when the server is merely unreachable", async () => {
    const client = await loadClient();
    client.setAccessToken("stale");

    const expired = vi.fn();
    client.onSessionExpired(expired);

    handler = (url) =>
      url.endsWith("/api/auth/refresh")
        ? json({ error: "boom" }, 503)
        : json({ error: "Session expired", code: "token_expired" }, 401);

    const err = await client.api.get("/api/thing").catch((e: unknown) => e);
    expect((err as { kind: string }).kind).toBe("unavailable");
    expect(expired).not.toHaveBeenCalled();
    // A 5xx says nothing about the session, so the token stays for a retry.
    expect(client.getAccessToken()).toBe("stale");
  });
});
