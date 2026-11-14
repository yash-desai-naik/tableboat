import { useSyncExternalStore } from "react";
import type { ThemeMode } from "../themes";
import type { Settings } from "./settings";

/**
 * Which mode (light or dark) the window shows, from the `theme` setting and Windows' preference.
 * Read from settings rather than `data-theme`, which a Settings hover preview changes for a moment.
 */

const query = () => window.matchMedia("(prefers-color-scheme: dark)");

export function prefersDark(): boolean {
  return query().matches;
}

function subscribe(listener: () => void) {
  const media = query();
  media.addEventListener("change", listener);
  return () => media.removeEventListener("change", listener);
}

/** Re-renders when Windows switches between light and dark. */
export function usePrefersDark(): boolean {
  return useSyncExternalStore(subscribe, prefersDark);
}

export function shownMode(theme: Settings["theme"], dark: boolean = prefersDark()): ThemeMode {
  return theme === "system" ? (dark ? "dark" : "light") : theme;
}
