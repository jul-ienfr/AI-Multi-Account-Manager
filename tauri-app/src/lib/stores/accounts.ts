import { writable, derived } from "svelte/store";
import type { AccountState, QuotaInfo } from "../types";
import * as api from "../tauri";

function createAccountsStore() {
  const { subscribe, set, update } = writable<AccountState[]>([]);
  return {
    subscribe,
    load: async () => {
      const accs = await api.getAccounts();
      set(accs);
    },
    switch: async (key: string) => {
      await api.switchAccount(key);
      update(accs => accs.map(a => ({ ...a, isActive: a.key === key })));
    },
    updateQuota: (key: string, quota: QuotaInfo) => {
      update(accs => accs.map(a => a.key === key ? { ...a, quota } : a));
    },
    refresh: async (key: string) => {
      await api.refreshAccount(key);
      const accs = await api.getAccounts();
      set(accs);
    },
    delete: async (key: string) => {
      await api.deleteAccount(key);
      update(accs => accs.filter(a => a.key !== key));
    },
    add: async (key: string, data: Parameters<typeof api.addAccount>[1]) => {
      await api.addAccount(key, data);
      const accs = await api.getAccounts();
      set(accs);
    },
    updateAccount: async (key: string, updates: { priority?: number; autoSwitchDisabled?: boolean; displayName?: string }) => {
      await api.updateAccount(key, updates);
      update(accs => accs.map(a => {
        if (a.key !== key) return a;
        return {
          ...a,
          data: {
            ...a.data,
            ...(updates.priority != null && { priority: updates.priority }),
            ...(updates.autoSwitchDisabled != null && { autoSwitchDisabled: updates.autoSwitchDisabled }),
            ...(updates.displayName != null && { displayName: updates.displayName }),
          },
        };
      }));
    },
  };
}

export const accounts = createAccountsStore();
export const activeAccount = derived(accounts, $a => $a.find(a => a.isActive) ?? null);
