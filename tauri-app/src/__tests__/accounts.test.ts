import { describe, it, expect, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

// Mock accounts to return test data
const mockAccounts = [
  { key: "alice", data: { name: "Alice", email: "alice@test.com", provider: "anthropic" }, isActive: true, quota: { tokens5h: 1000, limit5h: 45000000, tokens7d: 5000, limit7d: 180000000, phase: "Cruise", emaVelocity: 0 } },
  { key: "bob", data: { name: "Bob", provider: "gemini" }, isActive: false },
];

function mockFetchOnce(data: unknown, ok = true) {
  vi.mocked(fetch).mockResolvedValueOnce({
    ok,
    json: () => Promise.resolve(data),
  } as Response);
}

describe("accounts store", () => {
  beforeEach(() => {
    vi.mocked(fetch).mockReset();
  });

  it("load() fetches accounts and populates store", async () => {
    mockFetchOnce(mockAccounts);
    const { accounts } = await import("../lib/stores/accounts");
    await accounts.load();
    const state = get(accounts);
    expect(state).toHaveLength(2);
    expect(state[0].key).toBe("alice");
  });

  it("switch() updates isActive locally", async () => {
    mockFetchOnce(mockAccounts);
    const { accounts } = await import("../lib/stores/accounts");
    await accounts.load();
    mockFetchOnce(undefined);
    await accounts.switch("bob");
    const state = get(accounts);
    expect(state.find((a) => a.key === "bob")?.isActive).toBe(true);
  });

  it("delete() removes account from store", async () => {
    mockFetchOnce(mockAccounts);
    const { accounts } = await import("../lib/stores/accounts");
    await accounts.load();
    mockFetchOnce(undefined);
    await accounts.delete("bob");
    const state = get(accounts);
    expect(state).toHaveLength(1);
    expect(state[0].key).toBe("alice");
  });

  it("refresh() calls refreshAccount on correct REST path and reloads", async () => {
    mockFetchOnce(mockAccounts);
    const { accounts } = await import("../lib/stores/accounts");
    await accounts.load();
    mockFetchOnce(undefined); // POST /accounts/alice/refresh
    mockFetchOnce(mockAccounts); // GET /accounts
    await accounts.refresh("alice");
    expect(fetch).toHaveBeenCalledWith(
      "/ai-manager/admin/api/accounts/alice/refresh",
      expect.objectContaining({ method: "POST" })
    );
    expect(fetch).toHaveBeenCalledWith("/ai-manager/admin/api/accounts");
  });

  it("add() calls addAccount and reloads", async () => {
    mockFetchOnce(mockAccounts);
    const { accounts } = await import("../lib/stores/accounts");
    await accounts.load();
    mockFetchOnce(undefined); // POST /accounts
    mockFetchOnce([...mockAccounts, { key: "carol", data: { name: "Carol" }, isActive: false }]); // GET /accounts
    await accounts.add("carol", { name: "Carol" });
    const state = get(accounts);
    expect(state).toHaveLength(3);
  });

  it("activeAccount derived returns the active account", async () => {
    mockFetchOnce(mockAccounts);
    const { accounts, activeAccount } = await import("../lib/stores/accounts");
    await accounts.load();
    const active = get(activeAccount);
    expect(active?.key).toBe("alice");
  });
});
