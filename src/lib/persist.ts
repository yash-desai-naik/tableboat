import { invoke } from "@tauri-apps/api/core";
import { editorChanged, editors, hiddenEditorText } from "../editor/registry";
import { getSettings } from "./settings";
import { getWorkspace, restoreWorkspace, subscribeWorkspace, type SavedTab } from "./workspace";

/**
 * Open tabs are saved per connection (state/<connection-id>.json) a moment after each change,
 * and restored the next time that connection is opened. The last connection used is in
 * state/app.json so the main window can reopen it at start-up.
 */

interface SavedWorkspace {
  version: 1;
  database: string | null;
  activeIndex: number;
  tabs: SavedTab[];
}

const read = (name: string) => invoke<string | null>("state_read", { name });
const write = (name: string, contents: string) => invoke<void>("state_write", { name, contents });

let current: string | null = null; // connection id being saved
let timer: number | undefined;
let unsubscribe: (() => void) | null = null;

function snapshot(): SavedWorkspace {
  const ws = getWorkspace();
  const tabs: SavedTab[] = ws.tabs.map((t) =>
    t.type === "query"
      ? { type: "query", title: t.title, text: editors.get(t.id)?.text() ?? hiddenEditorText.get(t.id) ?? t.initialText }
      : { type: "object", database: t.database, kind: t.kind, name: t.name },
  );
  return { version: 1, database: ws.database, activeIndex: ws.tabs.findIndex((t) => t.id === ws.activeId), tabs };
}

export async function flushWorkspace() {
  window.clearTimeout(timer);
  if (!current) return;
  try {
    await write(current, JSON.stringify(snapshot()));
  } catch (err) {
    console.error("Could not save open tabs", err);
  }
}

function schedule() {
  window.clearTimeout(timer);
  timer = window.setTimeout(() => void flushWorkspace(), 800);
}

/** Start saving the workspace of this connection (stops saving the previous one). */
export function trackWorkspace(connectionId: string | null) {
  if (current && current !== connectionId) void flushWorkspace();
  unsubscribe?.();
  unsubscribe = null;
  current = connectionId;
  editorChanged.notify = connectionId ? schedule : () => {};
  if (connectionId) unsubscribe = subscribeWorkspace(schedule);
}

/** Restore a connection's saved tabs. Returns false when there was nothing to restore. */
export async function restoreFor(connectionId: string, fallbackDatabase: string | null): Promise<boolean> {
  if (!getSettings().restoreTabs) return false;
  try {
    const text = await read(connectionId);
    if (!text) return false;
    const saved = JSON.parse(text) as SavedWorkspace;
    if (saved.version !== 1 || !Array.isArray(saved.tabs)) return false;
    restoreWorkspace(saved.database ?? fallbackDatabase, saved.tabs, saved.activeIndex);
    return true;
  } catch {
    return false;
  }
}

// ---------- last connection ----------

export async function rememberLastConnection(connectionId: string | null) {
  try {
    await write("app", JSON.stringify({ lastConnectionId: connectionId }));
  } catch {
    /* not important */
  }
}

export async function lastConnection(): Promise<string | null> {
  try {
    const text = await read("app");
    return text ? ((JSON.parse(text) as { lastConnectionId?: string | null }).lastConnectionId ?? null) : null;
  } catch {
    return null;
  }
}
