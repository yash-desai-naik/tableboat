import data from "./themes.json";
import type { Settings } from "../lib/settings";

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

/** The theme the settings call for: mode from `theme` (`system` follows Windows), then that mode's pick. */
export function resolveTheme(settings: Pick<Settings, "theme" | "lightTheme" | "darkTheme">, prefersDark: boolean): ThemeData {
  const mode: ThemeMode = settings.theme === "system" ? (prefersDark ? "dark" : "light") : settings.theme;
  return pickTheme(mode, mode === "dark" ? settings.darkTheme : settings.lightTheme);
}

// ---------- colour maths (OKLab; same formulas as the generator and src-tauri/src/themes.rs) ----------
// Every value is computed to a plain colour here, never `color-mix()`: the result grid reads the
// variables for a canvas, which needs real colours.

type Vec3 = [number, number, number];

const hexRgb = (h: string): Vec3 => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255) as Vec3;
const rgbHex = (c: number[]) =>
  "#" + c.map((v) => Math.round(Math.max(0, Math.min(1, v)) * 255).toString(16).padStart(2, "0")).join("");
const lin = (v: number) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
const delin = (v: number) => (v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055);

function luminance(h: string): number {
  const [r, g, b] = hexRgb(h).map(lin);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const x = luminance(a);
  const y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

function toOklab(h: string): Vec3 {
  const [r, g, b] = hexRgb(h).map(lin);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function fromOklab([L, A, B]: Vec3): string {
  const l = (L + 0.3963377774 * A + 0.2158037573 * B) ** 3;
  const m = (L - 0.1055613458 * A - 0.0638541728 * B) ** 3;
  const s = (L - 0.0894841775 * A - 1.291485548 * B) ** 3;
  const rgb = [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0042360945 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
  return rgbHex(rgb.map((v) => delin(Math.max(0, Math.min(1, v)))));
}

/** `color-mix(in oklab, fg pct%, bg)` as `#rrggbb`. */
function mix(fg: string, pct: number, bg: string): string {
  const p = pct / 100;
  const a = toOklab(fg);
  const b = toOklab(bg);
  return fromOklab([0, 1, 2].map((i) => a[i] * p + b[i] * (1 - p)) as Vec3);
}

/** A hex colour at an alpha, as `rgba(r, g, b, a)`. */
function alpha(h: string, a: number): string {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

/** Moves a colour's OKLCH lightness by `d`, keeping hue and chroma. */
function shiftLightness(h: string, d: number): string {
  const [L, A, B] = toOklab(h);
  const C = Math.hypot(A, B);
  const H = Math.atan2(B, A);
  return fromOklab([Math.min(1, Math.max(0, L + d)), C * Math.cos(H), C * Math.sin(H)]);
}

/** Primary button hover: the fill a step darker, or lighter when darker would drop its text below 4.5:1. */
function accentHover(fill: string, on: string): string {
  const darker = shiftLightness(fill, -0.035);
  return contrast(on, darker) >= 4.5 ? darker : shiftLightness(fill, 0.035);
}

/**
 * Every CSS variable for a theme: seeds plus the shared derivations (docs/THEMES.md 5.2),
 * with `overrides` winning. Keep the names in step with the theme blocks in src/styles.css.
 */
export function themeVariables(theme: ThemeData): Record<string, string> {
  const dark = theme.mode === "dark";
  const { bg, chrome, sidebar, elevated } = theme.surface;
  const { primary: text, secondary, hint } = theme.text;
  const { danger, warning, success, info } = theme.status;
  const accent = theme.accent.text;

  let shadowRaised: string, shadowPopup: string, scrim: string;
  if (dark) {
    shadowRaised = "0 1px 2px rgba(0, 0, 0, 0.35)";
    shadowPopup = "0 12px 32px rgba(0, 0, 0, 0.5)";
    scrim = "rgba(0, 0, 0, 0.45)";
  } else {
    // A warm shadow on warm (cream) backgrounds, a cool one on cool backgrounds.
    const sh = parseInt(bg.slice(1, 3), 16) >= parseInt(bg.slice(5, 7), 16) ? "40, 30, 20" : "20, 30, 40";
    shadowRaised = `0 1px 2px rgba(${sh}, 0.14), 0 0 0 1px rgba(${sh}, 0.05)`;
    shadowPopup = `0 12px 32px rgba(${sh}, 0.16)`;
    scrim = alpha(text, 0.22);
  }

  const vars: Record<string, string> = {
    "--bg": bg,
    "--bg-chrome": chrome,
    "--bg-sidebar": sidebar,
    "--bg-elevated": elevated,
    "--bg-hover": mix(text, 6, chrome),
    "--bg-active": mix(text, 11, chrome),
    "--border": mix(text, 10, bg),
    "--border-strong": mix(text, 20, bg),
    "--text": text,
    "--text-2": secondary,
    "--text-3": hint,
    "--accent": accent,
    "--accent-text": theme.accent.on,
    "--accent-tint": mix(accent, 20, bg),
    "--accent-soft": alpha(accent, dark ? 0.22 : 0.14),
    "--selection": theme.selection || mix(accent, 30, bg),
    "--zebra": mix(text, 3, bg),
    "--control-fill": mix(text, 7, chrome),
    "--control-fill-hover": mix(text, 11, chrome),
    "--surface-raised": dark ? mix(text, 16, chrome) : elevated,
    "--shadow-raised": shadowRaised,
    "--danger": danger,
    "--scrim": scrim,
    "--shadow-popup": shadowPopup,
    "--scrollbar": mix(text, 22, bg),
    "--scrollbar-hover": mix(text, 32, bg),
    "--accent-fill": theme.accent.fill,
    "--accent-hover": accentHover(theme.accent.fill, theme.accent.on),
    "--focus-ring": accent,
    "--warning": warning,
    "--success": success,
    "--info": info,
    "--on-danger": theme.onDanger,
    "--danger-bg": mix(danger, 14, bg),
    "--success-bg": mix(success, 14, bg),
    "--warning-bg": mix(warning, 14, bg),
    "--grid-line": mix(text, 7, bg),
    "--edit-changed": mix(warning, 18, bg),
    "--edit-added": mix(success, 18, bg),
    "--edit-deleted": mix(danger, 18, bg),
    "--search-match": mix(warning, 30, bg),
    "--search-current": mix(warning, 55, bg),
    "--editor-gutter": hint,
    "--editor-gutter-active": secondary,
    "--editor-current-stmt": mix(text, 4, bg),
    "--editor-bracket": mix(text, 12, bg),
    "--editor-bracket-outline": hint,
  };
  for (const [k, v] of Object.entries(theme.syntax)) vars[`--syn-${k}`] = v;
  vars["--syn-invalid"] = danger;
  for (const [k, v] of Object.entries(theme.kind)) vars[`--kind-${k}`] = v;
  return { ...vars, ...theme.overrides };
}

// ---------- applying ----------

/** The theme the settings call for, re-applied when a preview ends. */
let applied: ThemeData | null = null;

function show(theme: ThemeData): void {
  const root = document.documentElement;
  let style = document.getElementById("theme-vars") as HTMLStyleElement | null;
  if (!style) {
    style = document.createElement("style");
    style.id = "theme-vars";
    document.head.appendChild(style);
  }
  const body = Object.entries(themeVariables(theme))
    .map(([k, v]) => `${k}:${v};`)
    .join("");
  // One rule, one text change: a switch is a single style recalculation. `:root:root:root` outranks
  // the fallback theme blocks in styles.css (`:root[data-theme]`) whatever order the sheets load in.
  style.textContent = `:root:root:root{${body}}`;
  root.dataset.theme = theme.mode;
  root.dataset.colorTheme = theme.id;
}

/** Show a theme on this window: sets `data-theme` (mode), `data-color-theme` (id) and the variables. */
export function applyTheme(theme: ThemeData): void {
  applied = theme;
  show(theme);
}

/**
 * Preview a theme without saving it (Settings hover). `null` restores the theme from settings.
 */
export function previewTheme(id: string | null): void {
  const theme = id ? themeById(id) : applied;
  if (theme) show(theme);
}
