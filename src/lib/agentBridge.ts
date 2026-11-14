import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { editors, hiddenEditorText } from "../editor/registry";
import { openQuery, runAll } from "../editor/actions";
import type { Command } from "./commands";
import { errorText, displayName } from "./connections";
import { pendingCount } from "./edits";
import type { ObjectKind } from "./meta";
import { getRunState } from "./query";
import { getSession } from "./session";
import { activeTab, getWorkspace, openObject, setDatabase } from "./workspace";

/**
 * Claude's hands in this window: the app's tool server (src-tauri/src/ai/host.rs) sends
 * `ui:request` events, this does them with the same functions the UI uses, and answers with
 * `ui_reply`. Questions to the user (for anything that changes data or loses work) are asked in
 * Rust before a request gets here.
 */

/** The command palette's list, kept current by App (so Claude sees exactly what the palette shows). */
let commands: Command[] = [];
export function publishCommands(list: Command[]) {
  commands = list;
}

const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n)}…` : s);

function editorText(tabId: string, initial: string): string {
  return editors.get(tabId)?.text() ?? hiddenEditorText.get(tabId) ?? initial;
}

/** What the window shows: connection, database, tabs, unsaved edits, the active tab's results. */
function state() {
  const session = getSession();
  const ws = getWorkspace();
  const tabs = ws.tabs.map((t, i) =>
    t.type === "object"
      ? {
          position: i + 1,
          active: t.id === ws.activeId,
          type: t.kind,
          database: t.database,
          name: t.name,
          preview: t.preview || undefined,
          unsavedEdits: pendingCount(t.id) || undefined,
        }
      : { position: i + 1, active: t.id === ws.activeId, type: "sql editor", title: t.title, text: clip(editorText(t.id, t.initialText), 4000) },
  );
  const active = activeTab();
  const run = active ? getRunState(active.id) : null;
  return {
    connection:
      session.status === "open"
        ? { name: displayName(session.info.connection), engine: session.info.server.product, version: session.info.server.version }
        : session.status === "connecting"
          ? `connecting to ${session.name}`
          : null,
    database: ws.database,
    tabs,
    activeTabResults: run
      ? {
          running: run.running || undefined,
          statements: run.outcomes.map((o) => ({
            sql: clip(o.sql, 300),
            rows: o.resultInfo?.rowCount,
            affectedRows: o.resultInfo ? undefined : (o.affectedRows ?? undefined),
            error: o.error ?? undefined,
            ms: Math.round(o.elapsedMs),
          })),
        }
      : null,
  };
}

/** New SQL tab, then run all of it once its editor has mounted. */
function openAndRun(sql: string) {
  openQuery(sql);
  const started = Date.now();
  const tick = () => {
    const tab = activeTab();
    if (tab?.type === "query" && editors.has(tab.id)) runAll();
    else if (Date.now() - started < 4000) window.setTimeout(tick, 50);
  };
  tick();
}

async function handle(action: string, args: Record<string, unknown>): Promise<unknown> {
  switch (action) {
    case "state":
      return state();
    case "commands":
      return commands
        .filter((c) => !c.hidden)
        .map((c) => ({ id: c.id, category: c.category, title: c.title, checked: c.checked, keys: c.keys?.[0] }));
    case "run": {
      const cmd = commands.find((c) => c.id === args.id);
      if (!cmd) throw new Error(`There's no command "${String(args.id)}" right now.`);
      await Promise.resolve(cmd.run());
      return null;
    }
    case "open_object":
      openObject({ database: String(args.database), name: String(args.name), kind: args.kind as ObjectKind }, true);
      return null;
    case "open_sql": {
      const sql = String(args.sql);
      if (args.run) openAndRun(sql);
      else if (args.mode === "insert") {
        const tab = activeTab();
        const ed = tab?.type === "query" ? editors.get(tab.id) : undefined;
        if (ed) ed.insert(sql);
        else openQuery(sql);
      } else openQuery(sql);
      return null;
    }
    case "use_database":
      setDatabase(String(args.database));
      return null;
    default:
      throw new Error(`Unknown action ${action}`);
  }
}

void getCurrentWebviewWindow().listen<{ id: string; action: string; args: Record<string, unknown> }>("ui:request", async (e) => {
  const { id, action, args } = e.payload;
  try {
    const value = await handle(action, args ?? {});
    await invoke("ui_reply", { id, ok: true, value: value ?? null, error: null });
  } catch (err) {
    await invoke("ui_reply", { id, ok: false, value: null, error: err instanceof Error ? err.message : errorText(err) });
  }
});
