import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";
import { errorText } from "./connections";
import { flushWorkspace } from "./persist";
import { updateSettings } from "./settings";

/** A newer Tableboat release (from src-tauri/src/update.rs). */
export interface UpdateInfo {
  version: string;
  current: string;
  /** Release notes, Markdown. */
  notes: string;
  date: string | null;
}

export type UpdatePhase = "idle" | "checking" | "uptodate" | "downloading" | "installing" | "error";

export interface UpdateState {
  phase: UpdatePhase;
  /** The update on offer, if any. */
  info: UpdateInfo | null;
  /** Show the notice (closed with Later / Skip; opened again by a new check). */
  noticeOpen: boolean;
  downloaded: number;
  total: number | null;
  error: string | null;
}

let state: UpdateState = { phase: "idle", info: null, noticeOpen: false, downloaded: 0, total: null, error: null };
const listeners = new Set<() => void>();

function set(patch: Partial<UpdateState>) {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

export function useUpdate(): UpdateState {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => state,
  );
}

// Every window hears about an update; a window opened later asks for one found earlier
// (and shows it in About, without popping the notice up again).
void listen<UpdateInfo>("update:available", (e) => set({ info: e.payload, noticeOpen: true, phase: "idle", error: null }));
void listen<{ downloaded: number; total: number | null }>("update:progress", (e) =>
  set({ phase: "downloading", downloaded: e.payload.downloaded, total: e.payload.total }),
);
void listen("update:installing", () => set({ phase: "installing" }));
// Claude's install_update, after the user agreed in the pane.
void listen("update:install-request", () => void installUpdate());
// Before the installer closes Tableboat, every window saves its tabs.
void listen("app:flush", () => void flushWorkspace());
void invoke<UpdateInfo | null>("update_status")
  .then((info) => info && !state.info && set({ info }))
  .catch(() => {});

let upToDateTimer: number | undefined;

/** "Check for updates": the notice shows the result either way. */
export async function checkForUpdates(): Promise<UpdateInfo | null> {
  set({ phase: "checking", error: null, noticeOpen: true });
  try {
    const info = await invoke<UpdateInfo | null>("update_check");
    if (info) {
      set({ phase: "idle", info, noticeOpen: true });
    } else {
      set({ phase: "uptodate", info: null });
      window.clearTimeout(upToDateTimer);
      upToDateTimer = window.setTimeout(() => state.phase === "uptodate" && set({ phase: "idle", noticeOpen: false }), 5000);
    }
    return info;
  } catch (err) {
    set({ phase: "error", error: errorText(err) });
    return null;
  }
}

/** Save every window's tabs, then download, verify and install; Tableboat restarts on the new version. */
export async function installUpdate() {
  set({ phase: "downloading", downloaded: 0, total: null, error: null, noticeOpen: true });
  try {
    await emit("app:flush");
    await new Promise((r) => window.setTimeout(r, 800));
    await invoke("update_install");
  } catch (err) {
    set({ phase: "error", error: errorText(err) });
  }
}

export function closeNotice() {
  set({ noticeOpen: false, phase: state.phase === "error" || state.phase === "uptodate" ? "idle" : state.phase });
}

/** Don't announce this version again (a manual check still shows it). */
export function skipUpdate() {
  if (state.info) updateSettings({ skippedUpdate: state.info.version });
  set({ noticeOpen: false });
}

export function openReleasesPage() {
  void invoke("open_releases_page").catch(() => {});
}
