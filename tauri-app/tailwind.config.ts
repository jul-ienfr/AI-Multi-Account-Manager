import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{svelte,ts}"],
  theme: {
    extend: {
      colors: {
        "bg-app": "var(--bg-app)",
        "bg-card": "var(--bg-card)",
        "bg-card-hover": "var(--bg-card-hover)",
        "bg-sidebar": "var(--bg-sidebar)",
        "fg-primary": "var(--fg-primary)",
        "fg-secondary": "var(--fg-secondary)",
        "fg-dim": "var(--fg-dim)",
        "fg-accent": "var(--fg-accent)",
        accent: "var(--accent)",
      },
    },
  },
  plugins: [],
} satisfies Config;
