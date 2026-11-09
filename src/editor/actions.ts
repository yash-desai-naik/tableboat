import { getSession } from "../lib/session";
import { getSettings } from "../lib/settings";
import { getRunState, queryApi, runForTab } from "../lib/query";
import { activeTab, getWorkspace, newQueryTab } from "../lib/workspace";
import { editors } from "./registry";

/** Shown in the results bar when something can't run (e.g. nothing under the cursor). */
export const editorNotices = new Map<string, string>();

function activeQueryTab() {
  const tab = activeTab();
  return tab?.type === "query" ? tab : undefined;
}

async function run(which: "current" | "all") {
  const tab = activeQueryTab();
  const session = getSession();
  if (!tab || session.status !== "open") return;
  const editor = editors.get(tab.id);
  if (!editor) return;
  const statements = which === "current" ? editor.currentStatements() : editor.allStatements();
  if (statements.length === 0) return;
  const s = getSettings();
  await runForTab(tab.id, {
    sessionId: session.info.sessionId,
    statements,
    database: session.info.connection.engine === "sqlite" ? null : getWorkspace().database,
    maxRows: s.maxRows,
    usePool: false,
    stopOnError: s.stopOnError,
  });
}

export const runCurrent = () => void run("current");
export const runAll = () => void run("all");

export function cancelActiveRun() {
  const tab = activeTab();
  if (!tab) return;
  const state = getRunState(tab.id);
  if (state.running && state.runId) void queryApi.cancel(state.runId);
}

export function formatActive(): string | null {
  const tab = activeQueryTab();
  if (!tab) return null;
  return editors.get(tab.id)?.format() ?? null;
}

/** Open a new SQL tab (optionally with text) and focus it. */
export function openQuery(text = "") {
  newQueryTab(text);
}

export function focusActiveEditor() {
  const tab = activeQueryTab();
  if (tab) editors.get(tab.id)?.focus();
}
