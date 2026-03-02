// Mock fetch global pour les tests
import { vi } from "vitest";

vi.stubGlobal("fetch", vi.fn());

// Mock localStorage
const store: Record<string, string> = {};
vi.stubGlobal("localStorage", {
  getItem: (key: string) => store[key] ?? null,
  setItem: (key: string, value: string) => { store[key] = value; },
  removeItem: (key: string) => { delete store[key]; },
  clear: () => { Object.keys(store).forEach((k) => delete store[k]); },
});

// Mock matchMedia
vi.stubGlobal("matchMedia", (query: string) => ({
  matches: query.includes("dark"),
  media: query,
  addEventListener: vi.fn(),
  removeEventListener: vi.fn(),
}));
