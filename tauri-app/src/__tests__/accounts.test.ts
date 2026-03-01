import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { get } from "svelte/store";

// Mock getAccounts to return test data
const mockAccounts = [
  { key: "alice", data: { name: "Alice", email: "alice@test.com", provider: "anthropic" }, isActive: true, quota: { tokens5h: 1000, limit5h: 45000000, tokens7d: 5000, limit7d: 180000000, phase: "Cruise", emaVelocity: 0 } },
  { key: "bob", data: { name: "Bob", provider: "gemini" }, isActive: false },
];

describe("accounts store", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("load() fetches accounts and populates store", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(mockAccounts);
    const { accounts } = await import("../lib/stores/accounts");
    await accounts.load();
    const state = get(accounts);
    expect(state).toHaveLength(2);
    expect(state[0].key).toBe("alice");
  });

  it("switch() updates isActive locally", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(mockAccounts);
    const { accounts } = await import("../lib/stores/accounts");
    await accounts.load();
    vi.mocked(invoke).mockResolvedValueOnce(undefined);
    await accounts.switch("bob");
    const state = get(accounts);
    expect(state.find((a) => a.key === "bob")?.isActive).toBe(true);
  });

  it("delete() removes account from store", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(mockAccounts);
    const { accounts } = await import("../lib/stores/accounts");
    await accounts.load();
    vi.mocked(invoke).mockResolvedValueOnce(undefined);
    await accounts.delete("bob");
    const state = get(accounts);
    expect(state).toHaveLength(1);
    expect(state[0].key).toBe("alice");
  });

  it("refresh() calls refreshAccount and reloads", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(mockAccounts);
    const { accounts } = await import("../lib/stores/accounts");
    await accounts.load();
    vi.mocked(invoke).mockResolvedValueOnce(undefined); // refreshAccount
    vi.mocked(invoke).mockResolvedValueOnce(mockAccounts); // getAccounts
    await accounts.refresh("alice");
    expect(invoke).toHaveBeenCalledWith("refresh_account", { key: "alice" });
    expect(invoke).toHaveBeenCalledWith("get_accounts");
  });

  it("add() calls addAccount and reloads", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(mockAccounts);
    const { accounts } = await import("../lib/stores/accounts");
    await accounts.load();
    vi.mocked(invoke).mockResolvedValueOnce(undefined); // addAccount
    vi.mocked(invoke).mockResolvedValueOnce([...mockAccounts, { key: "carol", data: { name: "Carol" }, isActive: false }]); // getAccounts
    await accounts.add("carol", { name: "Carol" });
    const state = get(accounts);
    expect(state).toHaveLength(3);
  });

  it("activeAccount derived returns the active account", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(mockAccounts);
    const { accounts, activeAccount } = await import("../lib/stores/accounts");
    await accounts.load();
    const active = get(activeAccount);
    expect(active?.key).toBe("alice");
  });
});
