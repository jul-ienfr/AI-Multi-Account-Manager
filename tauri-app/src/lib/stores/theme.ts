import { writable } from "svelte/store";

export type Theme = "dark" | "light" | "system";

function getSystemTheme(): "dark" | "light" {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function applyTheme(theme: Theme) {
  const resolved = theme === "system" ? getSystemTheme() : theme;
  document.documentElement.classList.toggle("light", resolved === "light");
}

function createThemeStore() {
  const stored = (localStorage.getItem("theme") as Theme) || "dark";
  const { subscribe, set } = writable<Theme>(stored);

  applyTheme(stored);

  // Écouter les changements système
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    const current = localStorage.getItem("theme") as Theme;
    if (current === "system") applyTheme("system");
  });

  return {
    subscribe,
    set: (value: Theme) => {
      localStorage.setItem("theme", value);
      applyTheme(value);
      set(value);
    },
  };
}

export const theme = createThemeStore();
