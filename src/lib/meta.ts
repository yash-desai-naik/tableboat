import { invoke } from "@tauri-apps/api/core";

export type ObjectKind = "table" | "view" | "function" | "procedure" | "trigger";

export const OBJECT_KINDS: { id: ObjectKind; label: string; plural: string }[] = [
  { id: "table", label: "Table", plural: "Tables" },
  { id: "view", label: "View", plural: "Views" },
  { id: "function", label: "Function", plural: "Functions" },
  { id: "procedure", label: "Procedure", plural: "Procedures" },
  { id: "trigger", label: "Trigger", plural: "Triggers" },
];

export interface DbObject {
  database: string;
  name: string;
  kind: ObjectKind;
  parent: string | null;
  rows: number | null;
  comment: string | null;
}

export interface Hit extends DbObject {
  score: number;
  indices: number[];
}

export function objectKey(o: { database: string; kind: ObjectKind; name: string }) {
  return `${o.database}\u0000${o.kind}\u0000${o.name}`;
}

export const metaApi = {
  databases: (sessionId: string, refresh = false) => invoke<string[]>("meta_databases", { sessionId, refresh }),
  objects: (sessionId: string, database: string, refresh = false) =>
    invoke<DbObject[]>("meta_objects", { sessionId, database, refresh }),
  search: (sessionId: string, query: string, database: string | null, sort: "score" | "name", limit: number) =>
    invoke<Hit[]>("meta_search", { sessionId, query, database, sort, limit }),
  ddl: (sessionId: string, database: string, kind: ObjectKind, name: string) =>
    invoke<string>("meta_ddl", { sessionId, database, kind, name }),
  refresh: (sessionId: string) => invoke<void>("meta_refresh", { sessionId }),
};

/** 1234 → "1.2k", 5_600_000 → "5.6M". */
export function compactCount(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  if (n < 1_000_000_000) return `${(n / 1_000_000).toFixed(n < 10_000_000 ? 1 : 0)}M`;
  return `${(n / 1_000_000_000).toFixed(1)}B`;
}

/** Quote an identifier for the session's SQL dialect. */
export function quoteIdent(name: string, engine: string): string {
  return engine === "sqlite" || engine === "postgres"
    ? `"${name.replace(/"/g, '""')}"`
    : `\`${name.replace(/`/g, "``")}\``;
}
