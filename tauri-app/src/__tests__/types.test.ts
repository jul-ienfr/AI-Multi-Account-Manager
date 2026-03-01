import { describe, it, expect } from "vitest";
import type { AccountState, QuotaInfo, ProxyStatus, Peer, Toast, Provider, RoutingStrategy } from "../lib/types";

describe("types", () => {
  it("AccountState has required fields", () => {
    const account: AccountState = {
      key: "test",
      data: { name: "Test", provider: "anthropic" },
      isActive: false,
    };
    expect(account.key).toBe("test");
    expect(account.isActive).toBe(false);
    expect(account.quota).toBeUndefined();
  });

  it("QuotaInfo supports all phases", () => {
    const phases = ["Cruise", "Watch", "Alert", "Critical"] as const;
    phases.forEach((phase) => {
      const quota: QuotaInfo = {
        tokens5h: 0, limit5h: 45000000,
        tokens7d: 0, limit7d: 180000000,
        phase, emaVelocity: 0,
      };
      expect(quota.phase).toBe(phase);
    });
  });

  it("ProxyStatus has all metrics fields", () => {
    const status: ProxyStatus = {
      running: true, port: 8080,
      uptimeSecs: 3600, requestsTotal: 100, requestsActive: 5,
    };
    expect(status.running).toBe(true);
    expect(status.pid).toBeUndefined();
  });

  it("Peer has all required fields", () => {
    const peer: Peer = {
      id: "peer1", host: "192.168.1.1", port: 9090, connected: true,
    };
    expect(peer.id).toBe("peer1");
    expect(peer.lastSeen).toBeUndefined();
  });

  it("Toast has correct structure", () => {
    const toast: Toast = {
      id: "1", type: "success", title: "Test", message: "OK",
    };
    expect(toast.type).toBe("success");
  });

  it("Provider enum has all 7 values", () => {
    const providers: Provider[] = ["anthropic", "gemini", "openai", "xai", "deepseek", "mistral", "groq"];
    expect(providers).toHaveLength(7);
  });

  it("RoutingStrategy enum has all 5 values", () => {
    const strategies: RoutingStrategy[] = ["priority", "quota-aware", "round-robin", "latency", "usage-based"];
    expect(strategies).toHaveLength(5);
  });
});
