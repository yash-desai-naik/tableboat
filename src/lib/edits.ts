import { invoke } from "@tauri-apps/api/core";
import { useSyncExternalStore } from "react";
import { keepTab, tabClosedHooks } from "./workspace";

/** Mirrors db::edit::EditValue. */
export type EditValue = { kind: "text"; value: string } | { kind: "null" } | { kind: "default" };

export interface TableColumn {
  name: string;
  typeName: string;
  nullable: boolean;
  default: string | null;
  primaryKey: boolean;
  autoIncrement: boolean;
  generated: boolean;
  binary: boolean;
}

type Row = (string | null)[];

/** Unsaved changes for one tab. Existing rows are identified by their primary key. */
export interface Pending {
  updates: Map<string, { original: Row; changes: Map<number, EditValue> }>;
  deletes: Map<string, Row>;
  inserts: { id: number; values: EditValue[] }[];
}

export interface EditContext {
  /** Result column index of each primary-key column. */
  keyIdx: number[];
  /** Table column info aligned with the result columns. */
  columns: (TableColumn | undefined)[];
  names: string[];
}

const pendings = new Map<string, Pending>();
const versions = new Map<string, number>();
const listeners = new Set<() => void>();
let insertSeq = 0;

let anyVersion = 0;

function emit(tabId: string) {
  versions.set(tabId, (versions.get(tabId) ?? 0) + 1);
  anyVersion++;
  for (const l of listeners) l();
}

/** Re-renders on any tab's pending-change update (e.g. the tab strip's unsaved dots). */
export function useAnyEditVersion(): number {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    () => anyVersion,
  );
}

tabClosedHooks.add((id) => {
  pendings.delete(id);
  versions.delete(id);
});

export function getPending(tabId: string): Pending {
  let p = pendings.get(tabId);
  if (!p) {
    p = { updates: new Map(), deletes: new Map(), inserts: [] };
    pendings.set(tabId, p);
  }
  return p;
}

/** Re-renders when the tab's pending changes change; returns a version number. */
export function useEditVersion(tabId: string): number {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    () => versions.get(tabId) ?? 0,
  );
}

export function pendingCount(tabId: string): number {
  const p = pendings.get(tabId);
  if (!p) return 0;
  let n = p.deletes.size + p.inserts.length;
  for (const [key, u] of p.updates) if (u.changes.size > 0 && !p.deletes.has(key)) n++;
  return n;
}

export function hasPending(tabId: string) {
  return pendingCount(tabId) > 0;
}

export function discardPending(tabId: string) {
  pendings.delete(tabId);
  emit(tabId);
}

export function rowKey(row: Row, keyIdx: number[]) {
  return JSON.stringify(keyIdx.map((i) => row[i]));
}

function changed(tabId: string) {
  // A tab with unsaved changes is never replaced as a preview tab.
  if (hasPending(tabId)) keepTab(tabId);
  emit(tabId);
}

function sameValue(v: EditValue, original: string | null) {
  if (v.kind === "null") return original === null;
  // Opening an editor on NULL and leaving it empty is not a change.
  if (v.kind === "text") return original === null ? false : v.value === original;
  return false;
}

/** Set a value in an existing row (by its loaded values) or a new row (by insert index). */
export function setExisting(tabId: string, ctx: EditContext, row: Row, col: number, value: EditValue) {
  const p = getPending(tabId);
  const key = rowKey(row, ctx.keyIdx);
  if (p.deletes.has(key)) return;
  if (value.kind === "text" && value.value === "" && row[col] === null) return;
  const entry = p.updates.get(key) ?? { original: row, changes: new Map() };
  if (sameValue(value, row[col])) entry.changes.delete(col);
  else entry.changes.set(col, value);
  if (entry.changes.size) p.updates.set(key, entry);
  else p.updates.delete(key);
  changed(tabId);
}

export function setInserted(tabId: string, index: number, col: number, value: EditValue) {
  const ins = getPending(tabId).inserts[index];
  if (!ins) return;
  ins.values[col] = value;
  changed(tabId);
}

export function addRow(tabId: string, ctx: EditContext) {
  getPending(tabId).inserts.push({ id: ++insertSeq, values: ctx.names.map(() => ({ kind: "default" }) as EditValue) });
  changed(tabId);
}

/** Copy rows (with any pending edits) into new rows; keys the server assigns stay DEFAULT. */
export function duplicateRows(tabId: string, ctx: EditContext, existing: Row[], insertIdx: number[]) {
  const p = getPending(tabId);
  const copy = (values: EditValue[]): EditValue[] =>
    values.map((v, i) => {
      const c = ctx.columns[i];
      if (c?.generated || (c?.primaryKey && c.autoIncrement)) return { kind: "default" };
      return v;
    });
  for (const row of existing) {
    const key = rowKey(row, ctx.keyIdx);
    const changes = p.updates.get(key)?.changes;
    const values: EditValue[] = row.map((v, i) => changes?.get(i) ?? (v === null ? { kind: "null" } : { kind: "text", value: v }));
    p.inserts.push({ id: ++insertSeq, values: copy(values) });
  }
  for (const i of insertIdx) {
    const ins = p.inserts[i];
    if (ins) p.inserts.push({ id: ++insertSeq, values: copy(ins.values) });
  }
  changed(tabId);
}

/** Mark existing rows for deletion (toggles back if all are already marked); drop new rows. */
export function deleteRows(tabId: string, ctx: EditContext, existing: Row[], insertIdx: number[]) {
  const p = getPending(tabId);
  const keys = existing.map((r) => rowKey(r, ctx.keyIdx));
  const allMarked = keys.length > 0 && keys.every((k) => p.deletes.has(k));
  existing.forEach((row, i) => {
    if (allMarked) p.deletes.delete(keys[i]);
    else p.deletes.set(keys[i], row);
  });
  const drop = new Set(insertIdx);
  p.inserts = p.inserts.filter((_, i) => !drop.has(i));
  changed(tabId);
}

/** What db::edit::save receives. */
export function toChanges(tabId: string, ctx: EditContext) {
  const p = getPending(tabId);
  const key = (row: Row) => ctx.keyIdx.map((i) => ({ column: ctx.names[i], value: row[i] }));
  const writable = (i: number) => !ctx.columns[i]?.generated;
  return {
    deletes: [...p.deletes.values()].map((row) => ({ key: key(row) })),
    updates: [...p.updates.entries()]
      .filter(([k, u]) => !p.deletes.has(k) && u.changes.size > 0)
      .map(([, u]) => ({
        key: key(u.original),
        set: [...u.changes.entries()].filter(([i]) => writable(i)).map(([i, value]) => ({ column: ctx.names[i], value })),
      })),
    inserts: p.inserts.map((ins) => ({
      values: ins.values.map((value, i) => ({ column: ctx.names[i], value })).filter((_, i) => writable(i)),
    })),
  };
}

export const editApi = {
  columns: (sessionId: string, database: string, table: string) =>
    invoke<TableColumn[]>("table_columns", { sessionId, database, table }),
  preview: (sessionId: string, database: string, table: string, changes: ReturnType<typeof toChanges>) =>
    invoke<string[]>("table_preview", { sessionId, database, table, changes }),
  save: (sessionId: string, database: string, table: string, changes: ReturnType<typeof toChanges>) =>
    invoke<number>("table_save", { sessionId, database, table, changes }),
};

/** Actions of the table tab that's open, for shortcuts (Ctrl+S, Ctrl+I…). */
export interface TableActions {
  reload(): void;
  save(): void;
  discard(): void;
  preview(): void;
  addRow(): void;
  duplicate(): void;
  deleteRows(): void;
}
export const tableActions = new Map<string, TableActions>();
