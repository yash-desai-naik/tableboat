// "@" context for the Claude pane: what can be mentioned, and the text sent with the message.
import { invoke } from "@tauri-apps/api/core";
import { api, displayName, type ConnectionSummary } from "../lib/connections";
import { editApi } from "../lib/edits";
import { metaApi, type DbObject, type ObjectKind } from "../lib/meta";
import { queryApi } from "../lib/query";
import { getSession } from "../lib/session";
import { getWorkspace } from "../lib/workspace";

export type Mention =
  | { kind: "object"; label: string; database: string; name: string; objectKind: ObjectKind }
  | { kind: "column"; label: string; database: string; table: string; column: string; type: string }
  | { kind: "database"; label: string; name: string }
  | { kind: "connection"; label: string; id: string; engine: string; database: string }
  | { kind: "file"; label: string; path: string; text: string; truncated: boolean }
  | { kind: "image"; label: string; mediaType: string; data: string };

/** One row of the "@" list: a mention to add, or an action (pick a file or an image). */
export interface Suggestion {
  key: string;
  group: string;
  label: string;
  detail?: string;
  mention?: Mention;
  action?: "file" | "image";
}

export function mentionKey(m: Mention): string {
  switch (m.kind) {
    case "object":
      return `o:${m.database}.${m.name}`;
    case "column":
      return `c:${m.database}.${m.table}.${m.column}`;
    case "database":
      return `d:${m.name}`;
    case "connection":
      return `n:${m.id}`;
    case "file":
      return `f:${m.path}`;
    case "image":
      return `i:${m.label}:${m.data.length}`;
  }
}

function current() {
  const s = getSession();
  if (s.status !== "open") return null;
  const engine = s.info.server.engine ?? s.info.connection.engine;
  const database = engine === "sqlite" ? "main" : getWorkspace().database;
  return { sessionId: s.info.sessionId, connectionId: s.info.connection.id, engine, database };
}

// Columns of a database, loaded once per session + database while the pane is used.
const columnCache = new Map<string, Promise<Record<string, { name: string; typeName: string }[]>>>();
function columnsOf(sessionId: string, database: string) {
  const key = `${sessionId}\u0000${database}`;
  let p = columnCache.get(key);
  if (!p) {
    p = queryApi.columns(sessionId, database).catch(() => ({}));
    columnCache.set(key, p);
  }
  return p;
}

const has = (text: string, q: string) => text.toLowerCase().includes(q.toLowerCase());

/** The "@" list for what was typed after "@": objects, columns (after "table."), databases, connections, files. */
export async function suggest(q: string): Promise<Suggestion[]> {
  const cur = current();
  const out: Suggestion[] = [];
  const isMongo = cur?.engine === "mongodb";

  // "orders.cu" → columns of orders starting/containing "cu".
  const dot = q.lastIndexOf(".");
  if (cur?.database && dot > 0) {
    const tableQ = q.slice(0, dot);
    const colQ = q.slice(dot + 1);
    const cols = await columnsOf(cur.sessionId, cur.database);
    let table = Object.keys(cols).find((t) => t.toLowerCase() === tableQ.toLowerCase());
    let list = table ? cols[table] : [];
    // MongoDB (and anything the bulk lookup missed): sample this one collection's fields.
    if (!list.length) {
      const one = await editApi.columns(cur.sessionId, cur.database, tableQ).catch(() => []);
      if (one.length) {
        table = tableQ;
        list = one.map((c) => ({ name: c.name, typeName: c.typeName }));
      }
    }
    if (table) {
      for (const c of list.filter((c) => has(c.name, colQ)).slice(0, 40)) {
        out.push({
          key: `c:${table}.${c.name}`,
          group: isMongo ? "Fields" : "Columns",
          label: `${table}.${c.name}`,
          detail: c.typeName,
          mention: { kind: "column", label: `${table}.${c.name}`, database: cur.database, table, column: c.name, type: c.typeName },
        });
      }
      return out;
    }
  }

  if (cur?.database) {
    let objects: DbObject[];
    try {
      objects = q ? await metaApi.search(cur.sessionId, q, cur.database, "score", 12) : (await metaApi.objects(cur.sessionId, cur.database)).slice(0, 12);
    } catch {
      objects = [];
    }
    for (const o of objects) {
      const kindLabel = isMongo && o.kind === "table" ? "collection" : o.kind;
      out.push({
        key: `o:${o.database}.${o.name}`,
        group: isMongo ? "Collections" : "Tables & objects",
        label: o.name,
        detail: `${kindLabel}${o.database !== cur.database ? ` · ${o.database}` : ""}`,
        mention: { kind: "object", label: o.name, database: o.database, name: o.name, objectKind: o.kind },
      });
    }
  }

  if (cur && cur.engine !== "sqlite") {
    const dbs = await metaApi.databases(cur.sessionId).catch(() => [] as string[]);
    for (const d of dbs.filter((d) => has(d, q)).slice(0, 5)) {
      out.push({
        key: `d:${d}`,
        group: isMongo ? "Databases" : "Databases / schemas",
        label: d,
        detail: d === cur.database ? "current" : undefined,
        mention: { kind: "database", label: d, name: d },
      });
    }
  }

  const conns: ConnectionSummary[] = await api.list().catch(() => []);
  for (const c of conns.filter((c) => c.id !== cur?.connectionId && has(`${displayName(c)} ${c.engine}`, q)).slice(0, 6)) {
    out.push({
      key: `n:${c.id}`,
      group: "Other connections",
      label: displayName(c),
      detail: c.engine === "mongodb" ? "MongoDB" : c.engine === "sqlite" ? "SQLite" : c.engine === "mariadb" ? "MariaDB" : "MySQL",
      mention: { kind: "connection", label: displayName(c), id: c.id, engine: c.engine, database: c.database },
    });
  }

  if (!q || has("file", q) || has("attach", q)) out.push({ key: "a:file", group: "Attach", label: "File…", detail: "text file", action: "file" });
  if (!q || has("image", q) || has("picture", q) || has("screenshot", q))
    out.push({ key: "a:image", group: "Attach", label: "Image…", detail: "PNG, JPEG, GIF, WebP", action: "image" });
  return out;
}

export interface AttachmentFile {
  kind: "text" | "image";
  name: string;
  path: string;
  text?: string;
  truncated?: boolean;
  mediaType?: string;
  data?: string;
  size: number;
}

/** Read a picked or dropped file into a mention (text file or image). */
export async function attachPath(path: string): Promise<Mention> {
  const a = await invoke<AttachmentFile>("read_attachment", { path });
  return a.kind === "image"
    ? { kind: "image", label: a.name, mediaType: a.mediaType!, data: a.data! }
    : { kind: "file", label: a.name, path: a.path, text: a.text!, truncated: !!a.truncated };
}

const IMAGE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/** An image from the clipboard (paste) as a mention; null when it isn't a supported image. */
export function imageFromBlob(file: File): Promise<Mention | null> {
  if (!IMAGE_TYPES.includes(file.type) || file.size > 5 * 1024 * 1024) return Promise.resolve(null);
  return new Promise((resolve) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result);
      const data = url.slice(url.indexOf(",") + 1);
      const ext = file.type.split("/")[1];
      resolve({ kind: "image", label: file.name && file.name !== "image.png" ? file.name : `pasted image.${ext}`, mediaType: file.type, data });
    };
    reader.onerror = () => resolve(null);
    reader.readAsDataURL(file);
  });
}

const MAX_CONTEXT = 200_000;

/**
 * The context sent with a message for its mentions (images go separately). Wrapped in a
 * <tableboat-context> block, which the chat hides when showing the message.
 */
export async function contextText(mentions: Mention[]): Promise<string> {
  const cur = current();
  const parts: string[] = [];
  for (const m of mentions) {
    try {
      switch (m.kind) {
        case "object": {
          let def = "";
          if (cur) def = await metaApi.ddl(cur.sessionId, m.database, m.objectKind, m.name).catch(() => "");
          const lang = cur?.engine === "mongodb" ? "js" : "sql";
          parts.push(`### ${m.objectKind === "table" && cur?.engine === "mongodb" ? "Collection" : m.objectKind[0].toUpperCase() + m.objectKind.slice(1)} \`${m.database}.${m.name}\`\n${def ? `\`\`\`${lang}\n${def.slice(0, 12_000)}\n\`\`\`` : "(definition not available; use get_schema)"}`);
          break;
        }
        case "column":
          parts.push(`### Column \`${m.database}.${m.table}.${m.column}\`: ${m.type}`);
          break;
        case "database": {
          let names: string[] = [];
          if (cur) names = (await metaApi.objects(cur.sessionId, m.name).catch(() => [] as DbObject[])).map((o) => `${o.name} (${o.kind})`);
          parts.push(`### Database \`${m.name}\`: ${names.length} objects\n${names.slice(0, 400).join(", ")}${names.length > 400 ? ", …" : ""}`);
          break;
        }
        case "connection":
          parts.push(
            `### Connection "${m.label}" (${m.engine}${m.database ? `, database ${m.database}` : ""})\nThe user added this connection to the chat: read it with get_schema / run_read_query and \`connection: "${m.label}"\` (read-only).`,
          );
          break;
        case "file": {
          const ext = m.path.split(".").pop()?.toLowerCase() ?? "";
          parts.push(`### File ${m.path}${m.truncated ? " (first 256 KB only)" : ""}\n\`\`\`${ext}\n${m.text}\n\`\`\``);
          break;
        }
        case "image":
          break;
      }
    } catch {
      /* one unavailable item doesn't stop the message */
    }
  }
  if (!parts.length) return "";
  let body = parts.join("\n\n");
  if (body.length > MAX_CONTEXT) body = `${body.slice(0, MAX_CONTEXT)}\n… (context cut at ${MAX_CONTEXT} characters)`;
  return `<tableboat-context>\nContext the user attached with @ mentions:\n\n${body}\n</tableboat-context>`;
}

/** The message as the user typed it, without the attached context (for showing saved chats). */
export function stripContext(text: string): { text: string; hadContext: boolean } {
  return {
    text: text.replace(/\s*<tableboat-context>[\s\S]*?<\/tableboat-context>\s*/g, "\n").trim(),
    hadContext: text.includes("<tableboat-context>"),
  };
}
