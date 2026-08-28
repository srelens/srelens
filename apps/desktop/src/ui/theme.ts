import { settingsStorage } from "@srelens/core";

export type ThemeMode = "dark" | "light" | "system";
export type ThemeName = "slate" | "srelens" | "graphite";

export interface Theme {
  name: ThemeName;
  mode: ThemeMode;
}

export interface ThemeOption {
  id: ThemeName;
  name: string;
  description: string;
  preview: string;
}

export const THEME_OPTIONS: ThemeOption[] = [
  {
    id: "slate",
    name: "Slate",
    description: "Cool blue operations",
    preview: "#60a5fa",
  },
  {
    id: "srelens",
    name: "srelens",
    description: "Teal infrastructure console",
    preview: "#25c7bd",
  },
  {
    id: "graphite",
    name: "Graphite",
    description: "Monochrome high contrast",
    preview: "#8b949e",
  },
];

const KEY = "fl-theme-v2";
const LEGACY_KEY = "fl-theme";
const LEGACY_BRAND = "free" + "lens";
const DEFAULT_THEME: Theme = { name: "slate", mode: "dark" };

function isThemeName(value: unknown): value is ThemeName {
  return typeof value === "string" && THEME_OPTIONS.some((theme) => theme.id === value);
}

function normalizeThemeName(value: unknown): ThemeName {
  if (value === LEGACY_BRAND) return "srelens";
  if (isThemeName(value)) return value;
  return DEFAULT_THEME.name;
}

function isThemeMode(value: unknown): value is ThemeMode {
  return value === "dark" || value === "light" || value === "system";
}

export function resolvedThemeMode(mode: ThemeMode): "dark" | "light" {
  if (mode !== "system") return mode;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/** Read the persisted theme (defaults to dark). */
export function getInitialTheme(): Theme {
  try {
    const stored = settingsStorage.getItem(KEY);
    if (stored) {
      const parsed = JSON.parse(stored) as Partial<Theme>;
      return {
        name: normalizeThemeName(parsed.name),
        mode: isThemeMode(parsed.mode) ? parsed.mode : DEFAULT_THEME.mode,
      };
    }

    const legacy = settingsStorage.getItem(LEGACY_KEY);
    if (legacy === "light" || legacy === "dark") {
      return { ...DEFAULT_THEME, mode: legacy };
    }
  } catch {
    /* storage unavailable, use the default below */
  }
  return DEFAULT_THEME;
}

/** Apply a theme to the document root and persist it. */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  const mode = resolvedThemeMode(theme.mode);

  // Set both conventions: app CSS keys off data attributes, shadcn/Tailwind
  // keys off the `dark` class. Keeping them in lockstep means one picker drives
  // both design systems.
  root.dataset.theme = theme.name;
  root.dataset.themeMode = mode;
  root.dataset.themePreference = theme.mode;
  root.classList.toggle("dark", mode === "dark");
  root.style.colorScheme = mode;
  try {
    settingsStorage.setItem(KEY, JSON.stringify(theme));
  } catch {
    /* storage unavailable, theme still applied for this session */
  }
}
