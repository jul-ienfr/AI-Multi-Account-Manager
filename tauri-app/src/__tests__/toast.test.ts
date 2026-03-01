import { describe, it, expect, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

describe("toast store", () => {
  beforeEach(() => {
    vi.resetModules();
  });

  it("starts with empty toasts", async () => {
    const { toasts } = await import("../lib/stores/toast");
    const state = get(toasts);
    expect(state).toEqual([]);
  });

  it("info() adds an info toast", async () => {
    const { toast, toasts } = await import("../lib/stores/toast");
    toast.info("Test", "Message");
    const state = get(toasts);
    expect(state).toHaveLength(1);
    expect(state[0].type).toBe("info");
    expect(state[0].title).toBe("Test");
  });

  it("error() adds an error toast", async () => {
    const { toast, toasts } = await import("../lib/stores/toast");
    toast.error("Erreur", "Something failed");
    const state = get(toasts);
    expect(state.some((t) => t.type === "error")).toBe(true);
  });

  it("dismiss() removes a specific toast", async () => {
    const { toast, toasts } = await import("../lib/stores/toast");
    toast.info("Keep");
    toast.info("Remove");
    let state = get(toasts);
    expect(state).toHaveLength(2);
    const idToRemove = state[1].id;
    toast.remove(idToRemove);
    state = get(toasts);
    expect(state).toHaveLength(1);
    expect(state[0].title).toBe("Keep");
  });
});
