import { invoke } from "@tauri-apps/api/core";
import { useSyncExternalStore } from "react";

export type Theme = "system" | "light" | "dark";
export type Density = "compact" | "default" | "comfortable";

/** Mirrors `Settings` in src-tauri/src/settings.rs. */
export interface Settings {
  theme: Theme;
  density: Density;
  uiZoom: number;
  editorFontSize: number;
  gridFontSize: number;
  wordWrap: boolean;
  showSidebar: boolean;
  showAiPane: boolean;
  sidebarWidth: number;
  aiPaneWidth: number;
  pageSize: number;
  sidebarKinds: string[];
  maxRows: number;
  stopOnError: boolean;
  resultsHeight: number;
  sortDescendingFirst: boolean;
  claudeModel: string;
  restoreTabs: boolean;
  reopenLastConnection: boolean;
}

let current: Settings | null = null;
const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export async function loadSettings(): Promise<Settings> {
  current = await invoke<Settings>("get_settings");
  emit();
  return current;
}

export function getSettings(): Settings {
  if (!current) throw new Error("Settings used before they were loaded");
  return current;
}

let pending: Partial<Settings> = {};
let timer: number | undefined;

/**
 * Apply a change immediately in the UI and save it to settings.json.
 * Pass `debounceMs` for rapid changes (e.g. dragging a pane edge) so the file
 * is written once at the end rather than on every mouse move.
 */
export function updateSettings(patch: Partial<Settings>, opts: { debounceMs?: number } = {}) {
  current = { ...getSettings(), ...patch };
  emit();
  pending = { ...pending, ...patch };
  window.clearTimeout(timer);
  timer = window.setTimeout(flush, opts.debounceMs ?? 0);
}

async function flush() {
  const patch = pending;
  pending = {};
  try {
    const saved = await invoke<Settings>("update_settings", { patch });
    // Keep any change made while the save was in flight.
    current = { ...saved, ...pending };
    emit();
  } catch (err) {
    console.error("Could not save settings", err);
  }
}

export function useSettings(): Settings {
  return useSyncExternalStore(subscribe, getSettings);
}
