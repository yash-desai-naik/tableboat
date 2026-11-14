import data from "./themes.json";

/**
 * Colour themes. The palettes live in themes.json (checked for contrast by `cargo test`, see
 * src-tauri/src/themes.rs and docs/THEMES.md); this module turns a theme into CSS variables.
 */

export type ThemeMode = "light" | "dark";

export interface ThemeData {
  id: string;
  name: string;
  mode: ThemeMode;
  /** Id of the theme it pairs with (opposite mode). */
  pair: string;
  highContrast: boolean;
  surface: { bg: string; chrome: string; sidebar: string; elevated: string };
  text: { primary: string; secondary: string; hint: string };
  accent: { text: string; fill: string; on: string };
  selection?: string;
  status: { danger: string; warning: string; success: string; info: string };
  onDanger: string;
  syntax: Record<"keyword" | "type" | "function" | "string" | "number" | "literal" | "property" | "operator" | "comment", string>;
  kind: Record<"table" | "view" | "function" | "procedure" | "trigger", string>;
  /** Exact CSS variable values that are not derived (the Tableboat themes keep their look). */
  overrides?: Record<string, string>;
  /** Colours nudged for contrast: palette value → stored value. */
  adjusted: { token: string; from: string; to: string }[];
}

export const THEMES: ThemeData[] = (data as { themes: ThemeData[] }).themes;

export const DEFAULT_THEME: Record<ThemeMode, string> = { light: "tableboat-light", dark: "tableboat-dark" };

export function themeById(id: string): ThemeData | undefined {
  return THEMES.find((t) => t.id === id);
}

export function themesFor(mode: ThemeMode): ThemeData[] {
  return THEMES.filter((t) => t.mode === mode);
}

/**
 * The theme to show for a mode: the chosen id when it exists and has that mode, else the default.
 * (The Rust side validates too; this guards against hand-edited settings files.)
 */
export function pickTheme(mode: ThemeMode, chosen: string | undefined): ThemeData {
  const t = chosen ? themeById(chosen) : undefined;
  return t && t.mode === mode ? t : themeById(DEFAULT_THEME[mode])!;
}

/**
 * Every CSS variable for a theme: seeds plus the shared derivations (docs/THEMES.md 5.2),
 * with `overrides` winning. IMPLEMENTED IN THE LOADER STEP.
 */
export function themeVariables(theme: ThemeData): Record<string, string> {
  void theme;
  return {};
}

/** Show a theme on this window: sets `data-theme` (mode), `data-color-theme` (id) and the variables. */
export function applyTheme(theme: ThemeData): void {
  document.documentElement.dataset.theme = theme.mode;
  document.documentElement.dataset.colorTheme = theme.id;
}

/**
 * Preview a theme without saving it (Settings hover). `null` restores the theme from settings.
 * IMPLEMENTED IN THE LOADER STEP.
 */
export function previewTheme(id: string | null): void {
  void id;
}
