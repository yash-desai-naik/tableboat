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

/** Raised after a format, so the tab's toolbar can show (or clear) an error. */
export const FORMAT_RESULT_EVENT = "tableboat:format-result";
export type FormatResult = { tabId: string; error: string | null };

/** Format the selection or the statement under the cursor ("current"), or the whole editor ("all"). */
export async function formatActive(scope: "current" | "all" = "current"): Promise<string | null> {
  const tab = activeQueryTab();
  const editor = tab ? editors.get(tab.id) : undefined;
  if (!tab || !editor) return null;
  const error = await editor.format(scope);
  window.dispatchEvent(new CustomEvent<FormatResult>(FORMAT_RESULT_EVENT, { detail: { tabId: tab.id, error } }));
  return error;
}

/** Open a new SQL tab (optionally with text) and focus it. */
export function openQuery(text = "") {
  newQueryTab(text);
}

export function focusActiveEditor() {
  const tab = activeQueryTab();
  if (tab) editors.get(tab.id)?.focus();
}
