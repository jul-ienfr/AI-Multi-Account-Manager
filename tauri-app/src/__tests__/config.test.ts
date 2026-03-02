import { describe, it, expect, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

function mockFetchOnce(data: unknown, ok = true) {
  vi.mocked(fetch).mockResolvedValueOnce({
    ok,
    json: () => Promise.resolve(data),
  } as Response);
}

describe("config store", () => {
  beforeEach(() => {
    vi.mocked(fetch).mockReset();
  });

  it("load() fetches config from backend via GET /config", async () => {
    const mockConfig = {
      refreshIntervalSecs: 60,
      adaptiveRefresh: true,
      proxy: { strategy: "priority", routerPort: 8080 },
      sync: { enabled: false, port: 9090 },
    };
    mockFetchOnce(mockConfig);
    const { config } = await import("../lib/stores/config");
    await config.load();
    const state = get(config);
    expect(state).toBeDefined();
    expect(fetch).toHaveBeenCalledWith("/ai-manager/admin/api/config");
  });

  it("save() calls PUT /config on backend", async () => {
    mockFetchOnce({ refreshIntervalSecs: 60 });
    const { config } = await import("../lib/stores/config");
    await config.load();
    mockFetchOnce(undefined);
    await config.save({ refreshIntervalSecs: 120 });
    expect(fetch).toHaveBeenCalledWith(
      "/ai-manager/admin/api/config",
      expect.objectContaining({
        method: "PUT",
        body: JSON.stringify({ refreshIntervalSecs: 120 }),
      })
    );
  });
});
