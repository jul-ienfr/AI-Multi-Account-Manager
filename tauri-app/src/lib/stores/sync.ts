import { writable } from "svelte/store";
import type { Peer } from "../types";
import * as api from "../tauri";

function createSyncStore() {
  const peers = writable<Peer[]>([]);
  const enabled = writable(false);
  return {
    peers: { subscribe: peers.subscribe },
    enabled: { subscribe: enabled.subscribe },
    load: async () => {
      const s = await api.getSyncStatus();
      enabled.set(s.enabled);
      const p = await api.getPeers();
      peers.set(p);
    },
    addPeer: async (host: string, port: number) => {
      await api.addPeer(host, port);
      const p = await api.getPeers();
      peers.set(p);
    },
    removePeer: async (id: string) => {
      await api.removePeer(id);
      const p = await api.getPeers();
      peers.set(p);
    },
    generateKey: async () => {
      return await api.generateSyncKey();
    },
    setKey: async (key: string) => {
      await api.setSyncKey(key);
    },
    testPeer: async (host: string, port: number) => {
      return await api.testPeerConnection(host, port);
    },
  };
}

export const syncStore = createSyncStore();
