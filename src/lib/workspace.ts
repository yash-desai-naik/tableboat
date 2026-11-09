import { useSyncExternalStore } from "react";
import type { ObjectKind } from "./meta";
import { dropAllRunStates, dropRunState } from "./query";

export interface ObjectTab {
  id: string;
  type: "object";
  database: string;
  kind: ObjectKind;
  name: string;
  /** A preview tab (italic) is replaced by the next object you open, until you keep it. */
  preview: boolean;
}

export interface QueryTab {
  id: string;
  type: "query";
  title: string;
  /** Text to start with (e.g. "Open in SQL Editor"). */
  initialText: string;
  preview: false;
}

export type Tab = ObjectTab | QueryTab;

interface WorkspaceState {
  /** The database the sidebar shows and queries run in. */
  database: string | null;
  tabs: Tab[];
  activeId: string | null;
}

let state: WorkspaceState = { database: null, tabs: [], activeId: null };
const listeners = new Set<() => void>();
let counter = 0;
let queryCounter = 0;

/** Called with a tab id when the tab closes, so its editor state etc. can be dropped. */
export const tabClosedHooks = new Set<(id: string) => void>();

function set(next: WorkspaceState) {
  state = next;
  for (const l of listeners) l();
}

/** Listen to every workspace change (used to save open tabs). */
export function subscribeWorkspace(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export interface SavedTab {
  type: "object" | "query";
  database?: string;
  kind?: ObjectKind;
  name?: string;
  title?: string;
  text?: string;
}

/** Bring back a connection's saved tabs (query tabs with their text). */
export function restoreWorkspace(database: string | null, saved: SavedTab[], activeIndex: number) {
  for (const t of state.tabs) for (const hook of tabClosedHooks) hook(t.id);
  dropAllRunStates();
  queryCounter = 0;
  const tabs: Tab[] = [];
  for (const s of saved) {
    if (s.type === "query") {
      const n = /^Query (\d+)$/.exec(s.title ?? "");
      if (n) queryCounter = Math.max(queryCounter, Number(n[1]));
      tabs.push({ id: `tab-${++counter}`, type: "query", title: s.title || `Query ${++queryCounter}`, initialText: s.text ?? "", preview: false });
    } else if (s.database && s.kind && s.name) {
      tabs.push({ id: `tab-${++counter}`, type: "object", database: s.database, kind: s.kind, name: s.name, preview: false });
    }
  }
  set({ database, tabs, activeId: tabs[Math.min(Math.max(0, activeIndex), tabs.length - 1)]?.id ?? null });
}

function tabClosed(id: string) {
  dropRunState(id);
  for (const hook of tabClosedHooks) hook(id);
}

export function useWorkspace(): WorkspaceState {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    () => state,
  );
}

export function getWorkspace() {
  return state;
}

export function activeTab(): Tab | undefined {
  return state.tabs.find((t) => t.id === state.activeId);
}

export function resetWorkspace(database: string | null) {
  for (const t of state.tabs) for (const hook of tabClosedHooks) hook(t.id);
  dropAllRunStates();
  queryCounter = 0;
  set({ database, tabs: [], activeId: null });
}

export function setDatabase(database: string) {
  if (state.database !== database) set({ ...state, database });
}

function insertTab(tab: Tab) {
  const at = state.tabs.findIndex((t) => t.id === state.activeId);
  const tabs = [...state.tabs];
  tabs.splice(at >= 0 ? at + 1 : tabs.length, 0, tab);
  set({ ...state, tabs, activeId: tab.id });
}

/**
 * Open an object. Without `keep`, it goes into the preview tab (replacing the previous
 * preview, as in VS Code and TablePlus); with `keep` it gets its own tab.
 */
export function openObject(obj: { database: string; kind: ObjectKind; name: string }, keep = false) {
  const same = state.tabs.find(
    (t): t is ObjectTab =>
      t.type === "object" && t.database === obj.database && t.kind === obj.kind && t.name === obj.name,
  );
  if (same) {
    const tabs = keep && same.preview ? state.tabs.map((t) => (t.id === same.id ? { ...same, preview: false } : t)) : state.tabs;
    set({ ...state, tabs, activeId: same.id });
    return;
  }
  const tab: ObjectTab = {
    id: `tab-${++counter}`,
    type: "object",
    database: obj.database,
    kind: obj.kind,
    name: obj.name,
    preview: !keep,
  };
  const previewIndex = keep ? -1 : state.tabs.findIndex((t) => t.preview);
  if (previewIndex >= 0) {
    const replaced = state.tabs[previewIndex];
    const tabs = [...state.tabs];
    tabs[previewIndex] = tab;
    set({ ...state, tabs, activeId: tab.id });
    tabClosed(replaced.id);
  } else {
    insertTab(tab);
  }
}

export function newQueryTab(initialText = "") {
  const tab: QueryTab = {
    id: `tab-${++counter}`,
    type: "query",
    title: `Query ${++queryCounter}`,
    initialText,
    preview: false,
  };
  insertTab(tab);
  return tab.id;
}

export function activateTab(id: string) {
  if (state.tabs.some((t) => t.id === id)) set({ ...state, activeId: id });
}

export function keepTab(id: string) {
  set({ ...state, tabs: state.tabs.map((t) => (t.id === id && t.type === "object" ? { ...t, preview: false } : t)) });
}

export function closeTab(id: string) {
  const index = state.tabs.findIndex((t) => t.id === id);
  if (index < 0) return;
  const tabs = state.tabs.filter((t) => t.id !== id);
  let activeId = state.activeId;
  if (activeId === id) {
    // Like VS Code: activate the neighbour to the right, else to the left.
    activeId = tabs[index]?.id ?? tabs[index - 1]?.id ?? null;
  }
  set({ ...state, tabs, activeId });
  tabClosed(id);
}

export function closeOtherTabs(id: string) {
  const closing = state.tabs.filter((t) => t.id !== id);
  set({ ...state, tabs: state.tabs.filter((t) => t.id === id), activeId: id });
  for (const t of closing) tabClosed(t.id);
}

export function closeAllTabs() {
  const closing = state.tabs;
  set({ ...state, tabs: [], activeId: null });
  for (const t of closing) tabClosed(t.id);
}

export function cycleTab(step: 1 | -1) {
  if (!state.tabs.length) return;
  const index = state.tabs.findIndex((t) => t.id === state.activeId);
  const next = (index + step + state.tabs.length) % state.tabs.length;
  set({ ...state, activeId: state.tabs[next].id });
}

export function activateTabAt(position: number) {
  // Ctrl+9 always goes to the last tab, as in browsers and VS Code.
  const tab = position >= 9 ? state.tabs[state.tabs.length - 1] : state.tabs[position - 1];
  if (tab) set({ ...state, activeId: tab.id });
}
