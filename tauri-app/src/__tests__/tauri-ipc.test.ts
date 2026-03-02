import { describe, it, expect, vi, beforeEach } from "vitest";

function mockFetchOnce(data: unknown, ok = true) {
  vi.mocked(fetch).mockResolvedValueOnce({
    ok,
    json: () => Promise.resolve(data),
  } as Response);
}

describe("tauri IPC wrapper", () => {
  beforeEach(() => {
    vi.mocked(fetch).mockReset();
  });

  it("getAccounts calls GET /accounts", async () => {
    mockFetchOnce([]);
    const { getAccounts } = await import("../lib/tauri");
    const result = await getAccounts();
    expect(fetch).toHaveBeenCalledWith("/ai-manager/admin/api/accounts");
    expect(result).toEqual([]);
  });

  it("switchAccount sends POST /accounts/:key/switch (no body)", async () => {
    mockFetchOnce(undefined);
    const { switchAccount } = await import("../lib/tauri");
    await switchAccount("alice");
    expect(fetch).toHaveBeenCalledWith(
      "/ai-manager/admin/api/accounts/alice/switch",
      expect.objectContaining({ method: "POST" })
    );
  });

  it("addAccount sends POST /accounts with data as body", async () => {
    mockFetchOnce(undefined);
    const { addAccount } = await import("../lib/tauri");
    await addAccount("bob", { name: "Bob", provider: "gemini" });
    expect(fetch).toHaveBeenCalledWith(
      "/ai-manager/admin/api/accounts",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ name: "Bob", provider: "gemini" }),
      })
    );
  });

  it("setConfig sends PUT /config with config as body", async () => {
    mockFetchOnce(undefined);
    const { setConfig } = await import("../lib/tauri");
    await setConfig({ refreshIntervalSecs: 120 });
    expect(fetch).toHaveBeenCalledWith(
      "/ai-manager/admin/api/config",
      expect.objectContaining({
        method: "PUT",
        body: JSON.stringify({ refreshIntervalSecs: 120 }),
      })
    );
  });

  it("startProxy sends POST /proxy/start with kind", async () => {
    mockFetchOnce(undefined);
    const { startProxy } = await import("../lib/tauri");
    await startProxy("router");
    expect(fetch).toHaveBeenCalledWith(
      "/ai-manager/admin/api/proxy/start",
      expect.objectContaining({ method: "POST", body: JSON.stringify({ kind: "router" }) })
    );
  });

  it("stopProxy sends POST /proxy/stop with kind", async () => {
    mockFetchOnce(undefined);
    const { stopProxy } = await import("../lib/tauri");
    await stopProxy("impersonator");
    expect(fetch).toHaveBeenCalledWith(
      "/ai-manager/admin/api/proxy/stop",
      expect.objectContaining({ method: "POST", body: JSON.stringify({ kind: "impersonator" }) })
    );
  });

  it("addPeer sends POST /peers with host and port", async () => {
    mockFetchOnce(undefined);
    const { addPeer } = await import("../lib/tauri");
    await addPeer("192.168.1.10", 9090);
    expect(fetch).toHaveBeenCalledWith(
      "/ai-manager/admin/api/peers",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ host: "192.168.1.10", port: 9090 }),
      })
    );
  });

  it("getQuotaHistory sends GET /monitoring/quota-history?key=...&period=...", async () => {
    mockFetchOnce([]);
    const { getQuotaHistory } = await import("../lib/tauri");
    const result = await getQuotaHistory("alice", "24h");
    expect(fetch).toHaveBeenCalledWith(
      "/ai-manager/admin/api/monitoring/quota-history?key=alice&period=24h"
    );
    expect(result).toEqual([]);
  });

  it("getLogs sends GET /monitoring/logs?filter=...", async () => {
    mockFetchOnce([]);
    const { getLogs } = await import("../lib/tauri");
    await getLogs("error");
    expect(fetch).toHaveBeenCalledWith(
      "/ai-manager/admin/api/monitoring/logs?filter=error"
    );
  });
});
