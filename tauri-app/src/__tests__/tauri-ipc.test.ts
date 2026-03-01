import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";

describe("tauri IPC wrapper", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("getAccounts calls invoke with correct command", async () => {
    vi.mocked(invoke).mockResolvedValueOnce([]);
    const { getAccounts } = await import("../lib/tauri");
    const result = await getAccounts();
    expect(invoke).toHaveBeenCalledWith("get_accounts");
    expect(result).toEqual([]);
  });

  it("switchAccount sends key parameter", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);
    const { switchAccount } = await import("../lib/tauri");
    await switchAccount("alice");
    expect(invoke).toHaveBeenCalledWith("switch_account", { key: "alice" });
  });

  it("addAccount sends key and data", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);
    const { addAccount } = await import("../lib/tauri");
    await addAccount("bob", { name: "Bob", provider: "gemini" });
    expect(invoke).toHaveBeenCalledWith("add_account", {
      key: "bob",
      data: { name: "Bob", provider: "gemini" },
    });
  });

  it("setConfig sends config object", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);
    const { setConfig } = await import("../lib/tauri");
    await setConfig({ refreshIntervalSecs: 120 });
    expect(invoke).toHaveBeenCalledWith("set_config", { config: { refreshIntervalSecs: 120 } });
  });

  it("startProxy sends kind parameter", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);
    const { startProxy } = await import("../lib/tauri");
    await startProxy("router");
    expect(invoke).toHaveBeenCalledWith("start_proxy", { kind: "router" });
  });

  it("stopProxy sends kind parameter", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);
    const { stopProxy } = await import("../lib/tauri");
    await stopProxy("impersonator");
    expect(invoke).toHaveBeenCalledWith("stop_proxy", { kind: "impersonator" });
  });

  it("addPeer sends host and port", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);
    const { addPeer } = await import("../lib/tauri");
    await addPeer("192.168.1.10", 9090);
    expect(invoke).toHaveBeenCalledWith("add_peer", { host: "192.168.1.10", port: 9090 });
  });

  it("getQuotaHistory sends key and period", async () => {
    vi.mocked(invoke).mockResolvedValueOnce([]);
    const { getQuotaHistory } = await import("../lib/tauri");
    await getQuotaHistory("alice", "24h");
    expect(invoke).toHaveBeenCalledWith("get_quota_history", { key: "alice", period: "24h" });
  });

  it("getLogs sends optional filter", async () => {
    vi.mocked(invoke).mockResolvedValueOnce([]);
    const { getLogs } = await import("../lib/tauri");
    await getLogs("error");
    expect(invoke).toHaveBeenCalledWith("get_logs", { filter: "error" });
  });
});
