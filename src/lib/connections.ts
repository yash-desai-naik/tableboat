import { invoke } from "@tauri-apps/api/core";

/** Mirrors src-tauri/src/conn/model.rs */
export type Engine = "mysql" | "mariadb" | "postgres" | "sqlite" | "mongodb";
export type SslMode = "disabled" | "preferred" | "required" | "verify-ca" | "verify-identity";
export type SshAuth = "password" | "key" | "agent";

export interface Connection {
  id: string;
  name: string;
  engine: Engine;
  color: string;
  tag: string | null;
  host: string;
  port: number | null;
  user: string;
  database: string;
  file: string | null;
  ssl: { mode: SslMode; caFile: string | null; certFile: string | null; keyFile: string | null };
  ssh: { enabled: boolean; host: string; port: number; user: string; auth: SshAuth; keyFile: string | null };
  savePassword: boolean;
  options: Record<string, string>;
  createdAt: number;
  updatedAt: number;
}

export interface ConnectionSummary extends Connection {
  hasPassword: boolean;
  hasSshPassword: boolean;
  hasSshPassphrase: boolean;
}

/** `undefined` = keep what's stored, "" = remove, anything else = save. */
export interface Secrets {
  password?: string;
  sshPassword?: string;
  sshPassphrase?: string;
}

export interface ServerInfo {
  engine: Engine | null;
  product: string;
  version: string;
  fullVersion: string;
  database: string | null;
  tls: boolean;
  ssh: boolean;
}

export interface SessionInfo {
  sessionId: string;
  connection: Connection;
  server: ServerInfo;
}

export const ENGINES: { id: Engine; label: string; badge: string; ready: boolean; defaultPort: number | null }[] = [
  { id: "mysql", label: "MySQL", badge: "My", ready: true, defaultPort: 3306 },
  { id: "mariadb", label: "MariaDB", badge: "Ma", ready: true, defaultPort: 3306 },
  { id: "sqlite", label: "SQLite", badge: "Sl", ready: true, defaultPort: null },
  { id: "postgres", label: "PostgreSQL", badge: "Pg", ready: false, defaultPort: 5432 },
  { id: "mongodb", label: "MongoDB", badge: "Mg", ready: true, defaultPort: 27017 },
];

export function engineInfo(engine: Engine) {
  return ENGINES.find((e) => e.id === engine)!;
}

export const COLOR_PRESETS = ["#6b7280", "#1f6feb", "#16a34a", "#d97706", "#dc2626", "#9333ea", "#0891b2", "#db2777"];

export const TAGS: { id: string; label: string; short: string; color: string }[] = [
  { id: "local", label: "Local", short: "LOCAL", color: "#6b7280" },
  { id: "development", label: "Development", short: "DEV", color: "#1f6feb" },
  { id: "testing", label: "Testing", short: "TEST", color: "#9333ea" },
  { id: "staging", label: "Staging", short: "STAGING", color: "#d97706" },
  { id: "production", label: "Production", short: "PROD", color: "#dc2626" },
];

export function tagInfo(tag: string | null) {
  if (!tag) return null;
  return TAGS.find((t) => t.id === tag.toLowerCase()) ?? { id: tag, label: tag, short: tag.toUpperCase(), color: "#6b7280" };
}

export function newConnection(engine: Engine = "mysql"): Connection {
  return {
    id: "",
    name: "",
    engine,
    color: COLOR_PRESETS[1],
    tag: null,
    host: "127.0.0.1",
    port: null,
    user: engine === "mysql" || engine === "mariadb" ? "root" : "",
    database: "",
    file: null,
    ssl: { mode: "preferred", caFile: null, certFile: null, keyFile: null },
    ssh: { enabled: false, host: "", port: 22, user: "", auth: "password", keyFile: null },
    savePassword: true,
    options: {},
    createdAt: 0,
    updatedAt: 0,
  };
}

export function displayName(c: Connection): string {
  if (c.name.trim()) return c.name.trim();
  if (c.engine === "sqlite") return c.file?.split(/[\\/]/).pop() ?? "SQLite";
  return c.database ? `${c.database}@${c.host}` : c.host;
}

/** "root@127.0.0.1:3306/shop" or the SQLite file path. */
export function describeTarget(c: Connection): string {
  if (c.engine === "sqlite") return c.file ?? "";
  const port = c.port ?? engineInfo(c.engine).defaultPort;
  const user = c.user ? `${c.user}@` : "";
  const db = c.database ? `/${c.database}` : "";
  return `${user}${c.host}${port ? `:${port}` : ""}${db}`;
}

export const api = {
  list: () => invoke<ConnectionSummary[]>("connections_list"),
  save: (connection: Connection, secrets: Secrets) =>
    invoke<ConnectionSummary>("connection_save", { connection, secrets }),
  remove: (id: string) => invoke<void>("connection_delete", { id }),
  duplicate: (id: string) => invoke<ConnectionSummary>("connection_duplicate", { id }),
  parseUrl: (input: string) => invoke<{ connection: Connection; secrets: Secrets }>("connection_parse_url", { input }),
  toUrl: (id: string, includePasswords: boolean) => invoke<string>("connection_url", { id, includePasswords }),
  exportFile: (path: string, includePasswords: boolean) =>
    invoke<number>("connections_export", { path, includePasswords }),
  importFile: (path: string) => invoke<{ imported: number; errors: string[] }>("connections_import", { path }),
  test: (connection: Connection, secrets: Secrets) => invoke<ServerInfo>("connection_test", { connection, secrets }),
  createSqliteFile: (path: string) => invoke<void>("sqlite_create_file", { path }),
  openSession: (id: string, password?: string) => invoke<SessionInfo>("session_open", { id, password }),
  closeSession: (sessionId: string) => invoke<void>("session_close", { sessionId }),
  newWindow: (connect?: string) => invoke<void>("window_new", { connect }),
};

export function errorText(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}
