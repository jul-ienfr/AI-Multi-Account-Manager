import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { get } from "svelte/store";

describe("config store", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("load() fetches config from backend", async () => {
    const mockConfig = {
      refreshIntervalSecs: 60,
      adaptiveRefresh: true,
      proxy: { strategy: "priority", routerPort: 8080 },
      sync: { enabled: false, port: 9090 },
    };
    vi.mocked(invoke).mockResolvedValueOnce(mockConfig);
    const { config } = await import("../lib/stores/config");
    await config.load();
    const state = get(config);
    expect(state).toBeDefined();
  });

  it("save() calls set_config on backend", async () => {
    vi.mocked(invoke).mockResolvedValueOnce({ refreshIntervalSecs: 60 });
    const { config } = await import("../lib/stores/config");
    await config.load();
    vi.mocked(invoke).mockResolvedValueOnce(undefined);
    await config.save({ refreshIntervalSecs: 120 });
    expect(invoke).toHaveBeenCalledWith("set_config", { config: { refreshIntervalSecs: 120 } });
  });
});
