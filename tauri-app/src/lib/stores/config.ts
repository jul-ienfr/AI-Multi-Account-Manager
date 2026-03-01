import { writable } from "svelte/store";
import type { AppConfig } from "../types";
import * as api from "../tauri";

function createConfigStore() {
  const { subscribe, set, update } = writable<AppConfig | null>(null);
  return {
    subscribe,
    load: async () => {
      const c = await api.getConfig();
      set(c);
    },
    save: async (partial: Partial<AppConfig>) => {
      await api.setConfig(partial);
      update(c => c ? { ...c, ...partial } as AppConfig : null);
    }
  };
}

export const config = createConfigStore();
