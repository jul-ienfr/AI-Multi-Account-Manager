import { writable } from "svelte/store";
import type { ProxyStatus, ProxyInstanceState, ProxyInstanceConfig } from "../types";
import * as api from "../tauri";

const defaultStatus: ProxyStatus = { running: false, port: 0, uptimeSecs: 0, requestsTotal: 0, requestsActive: 0 };

// Legacy proxy store (backward compat)
function createProxyStore() {
  const { subscribe, set } = writable<{ router: ProxyStatus; impersonator: ProxyStatus }>({
    router: { ...defaultStatus, port: 18080 },
    impersonator: { ...defaultStatus, port: 18081 }
  });
  return {
    subscribe,
    load: async () => {
      const s = await api.getProxyStatus();
      set(s);
    },
    start: async (kind: "router" | "impersonator") => {
      await api.startProxy(kind);
      const s = await api.getProxyStatus();
      set(s);
    },
    stop: async (kind: "router" | "impersonator") => {
      await api.stopProxy(kind);
      const s = await api.getProxyStatus();
      set(s);
    },
    restart: async (kind: "router" | "impersonator") => {
      await api.restartProxy(kind);
      const s = await api.getProxyStatus();
      set(s);
    }
  };
}

// Dynamic proxy instances store
function createProxyInstancesStore() {
  const { subscribe, set } = writable<ProxyInstanceState[]>([]);
  return {
    subscribe,
    load: async () => {
      const instances = await api.probeProxyInstances();
      set(instances);
    },
    probe: async () => {
      const instances = await api.probeProxyInstances();
      set(instances);
    },
    add: async (config: ProxyInstanceConfig) => {
      await api.addProxyInstance(config);
      const instances = await api.getProxyInstances();
      set(instances);
    },
    update: async (id: string, updates: Partial<ProxyInstanceConfig>) => {
      await api.updateProxyInstance(id, updates);
      const instances = await api.probeProxyInstances();
      set(instances);
    },
    remove: async (id: string) => {
      await api.deleteProxyInstance(id);
      const instances = await api.getProxyInstances();
      set(instances);
    },
    start: async (id: string) => {
      await api.startProxyInstance(id);
      const instances = await api.getProxyInstances();
      set(instances);
    },
    stop: async (id: string) => {
      await api.stopProxyInstance(id);
      const instances = await api.getProxyInstances();
      set(instances);
    },
    restart: async (id: string) => {
      await api.restartProxyInstance(id);
      await new Promise((r) => setTimeout(r, 500));
      const instances = await api.probeProxyInstances();
      set(instances);
    },
  };
}

export const proxyStatus = createProxyStore();
export const proxyInstances = createProxyInstancesStore();
