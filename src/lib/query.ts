import { invoke } from "@tauri-apps/api/core";
import { useSyncExternalStore } from "react";

export interface ColumnMeta {
  name: string;
  typeName: string;
  table: string | null;
  numeric: boolean;
  binary: boolean;
  primaryKey: boolean;
}

export interface ResultInfo {
  id: string;
  columns: ColumnMeta[];
  rowCount: number;
  truncated: boolean;
  /** MongoDB: the rows are documents (the document view is available). */
  documents: boolean;
}

export interface StatementOutcome {
  index: number;
  sql: string;
  elapsedMs: number;
  resultInfo: ResultInfo | null;
  affectedRows: number | null;
  lastInsertId: number | null;
  warnings: number;
  info: string | null;
  error: string | null;
  cancelled: boolean;
}

export type CopyFormat = "tsv" | "tsv-header" | "csv" | "csv-header" | "json" | "markdown" | "insert" | "xlsx";
export type Dialect = "mysql" | "sqlite";

export const queryApi = {
  run: (args: {
    sessionId: string;
    runId: string;
    statements: string[];
    database: string | null;
    maxRows: number;
    usePool: boolean;
    stopOnError: boolean;
  }) => invoke<StatementOutcome[]>("query_run", args),
  cancel: (runId: string) => invoke<boolean>("query_cancel", { runId }),
  rows: (resultId: string, start: number, count: number) =>
    invoke<(string | null)[][]>("result_rows", { resultId, start, count }),
  sort: (resultId: string, column: number | null, descending: boolean) =>
    invoke<number>("result_sort", { resultId, column, descending }),
  filter: (resultId: string, text: string | null) => invoke<number>("result_filter", { resultId, text }),
  copy: (args: {
    resultId: string;
    ranges: [number, number][] | null;
    columns: number[] | null;
    format: CopyFormat;
    dialect: Dialect;
    table: string | null;
  }) => invoke<string>("result_copy", args),
  exportFile: (args: { resultId: string; path: string; format: CopyFormat; dialect: Dialect; table: string | null }) =>
    invoke<number>("result_export", args),
  /** Documents of the current view as canonical Extended JSON. */
  documents: (resultId: string, start: number, count: number) =>
    invoke<unknown[]>("result_documents", { resultId, start, count }),
  /** One document as text: "shell" (mongosh syntax), "json" (relaxed Extended JSON). */
  documentText: (resultId: string, index: number, format: "shell" | "json" | "new") =>
    invoke<string>("result_document_text", { resultId, index, format }),
  free: (resultIds: string[]) => (resultIds.length ? invoke<void>("result_free", { resultIds }) : Promise.resolve()),
  columns: (sessionId: string, database: string) =>
    invoke<Record<string, { name: string; typeName: string }[]>>("meta_columns", { sessionId, database }),
};

/** Saving whole documents (MongoDB collection views). `id` is the `_id` as canonical Extended JSON. */
export const documentApi = {
  replace: (sessionId: string, database: string, collection: string, id: unknown, text: string) =>
    invoke<void>("document_replace", { sessionId, database, collection, id, text }),
  remove: (sessionId: string, database: string, collection: string, id: unknown) =>
    invoke<void>("document_delete", { sessionId, database, collection, id }),
  insert: (sessionId: string, database: string, collection: string, text: string) =>
    invoke<string>("document_insert", { sessionId, database, collection, text }),
};

export function dialectOf(engine: string): Dialect {
  return engine === "sqlite" ? "sqlite" : "mysql";
}

export function newRunId() {
  return crypto.randomUUID();
}

// ---------- per-tab run state (survives switching tabs) ----------

export interface RunState {
  runId: string | null;
  running: boolean;
  startedAt: number;
  outcomes: StatementOutcome[];
  /** Selected result tab: an outcome index, or -1 for Messages. */
  active: number;
  /** What ran last, so it can be run again (Re-run, Ctrl+R). */
  lastRun?: RunArgs;
}

export interface RunArgs {
  sessionId: string;
  statements: string[];
  database: string | null;
  maxRows: number;
  usePool: boolean;
  stopOnError: boolean;
}

const READ_SQL = /^\s*(select|with|show|describe|desc|explain|pragma|values|table)\b/i;
const MONGO_READ = /^\s*(show\s+\w+|db\.[\s\S]*\.(find|findOne|aggregate|countDocuments|count|estimatedDocumentCount|distinct|getIndexes)\s*\()/i;
const MONGO_WRITE = /\$out\b|\$merge\b|\.(insert|update|replace|delete|remove|drop|create|rename)\w*\s*\(/i;

/** True when every statement only reads (so a refresh may safely run it again). */
export function isReadOnlyRun(statements: string[]): boolean {
  return statements.every((s) => {
    const sql = s.replace(/^(\s*(--[^\n]*|#[^\n]*|\/\/[^\n]*|\/\*[\s\S]*?\*\/))+/, "");
    if (/^\s*db\.|^\s*show\s/i.test(sql)) return MONGO_READ.test(sql) && !MONGO_WRITE.test(sql);
    // "WITH … DELETE/UPDATE" exists in MySQL 8: treat any write keyword as a write.
    return READ_SQL.test(sql) && !/\b(insert|update|delete|replace|drop|alter|create|truncate|grant|revoke)\b/i.test(sql.replace(/'(?:[^']|'')*'|"(?:[^"]|"")*"|`[^`]*`/g, "''"));
  });
}

const EMPTY: RunState = { runId: null, running: false, startedAt: 0, outcomes: [], active: -1 };
const runStates = new Map<string, RunState>();
const listeners = new Set<() => void>();

function emit() {
  for (const l of listeners) l();
}

export function getRunState(tabId: string): RunState {
  return runStates.get(tabId) ?? EMPTY;
}

export function setRunState(tabId: string, patch: Partial<RunState>) {
  runStates.set(tabId, { ...getRunState(tabId), ...patch });
  emit();
}

export function useRunState(tabId: string): RunState {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    () => getRunState(tabId),
  );
}

export function resultIds(state: RunState): string[] {
  return state.outcomes.flatMap((o) => (o.resultInfo ? [o.resultInfo.id] : []));
}

/** Forget a tab's results (frees the rows held in Rust). */
export function dropRunState(tabId: string) {
  const state = runStates.get(tabId);
  if (!state) return;
  if (state.runId && state.running) void queryApi.cancel(state.runId);
  void queryApi.free(resultIds(state));
  runStates.delete(tabId);
  emit();
}

export function dropAllRunStates() {
  for (const id of [...runStates.keys()]) dropRunState(id);
}

/** Run statements for a tab, replacing (and freeing) its previous results. */
export async function runForTab(tabId: string, args: RunArgs): Promise<StatementOutcome[]> {
  const previous = getRunState(tabId);
  if (previous.running && previous.runId) await queryApi.cancel(previous.runId);
  void queryApi.free(resultIds(previous));
  const runId = newRunId();
  setRunState(tabId, { runId, running: true, startedAt: performance.now(), outcomes: [], active: -1, lastRun: args });
  try {
    const outcomes = await queryApi.run({ ...args, runId });
    // Show the first result with rows; otherwise the messages.
    const firstResult = outcomes.findIndex((o) => o.resultInfo);
    const firstError = outcomes.findIndex((o) => o.error && !o.cancelled);
    const active = firstError >= 0 && (firstResult < 0 || firstError < firstResult) ? -1 : firstResult;
    if (getRunState(tabId).runId === runId) {
      setRunState(tabId, { running: false, outcomes, active });
    } else {
      void queryApi.free(outcomes.flatMap((o) => (o.resultInfo ? [o.resultInfo.id] : [])));
    }
    return outcomes;
  } catch (err) {
    const message = typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
    const failed: StatementOutcome = {
      index: 0,
      sql: args.statements[0] ?? "",
      elapsedMs: 0,
      resultInfo: null,
      affectedRows: null,
      lastInsertId: null,
      warnings: 0,
      info: null,
      error: message,
      cancelled: false,
    };
    setRunState(tabId, { running: false, outcomes: [failed], active: -1 });
    return [failed];
  }
}

/** Run a tab's last statements again (keeps the selected result tab when it still exists). */
export async function rerunTab(tabId: string): Promise<boolean> {
  const state = getRunState(tabId);
  if (!state.lastRun || state.running) return false;
  const active = state.active;
  await runForTab(tabId, state.lastRun);
  const now = getRunState(tabId);
  if (active >= 0 && now.outcomes[active]?.resultInfo) setRunState(tabId, { active });
  return true;
}

export function formatElapsed(ms: number): string {
  if (ms < 1) return `${Math.round(ms * 1000)} µs`;
  if (ms < 1000) return `${ms < 10 ? ms.toFixed(1) : Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}
