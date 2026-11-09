import { invoke } from "@tauri-apps/api/core";
import { useSyncExternalStore } from "react";
import { refreshAll } from "./refresh";

export type ChatItem =
  | { kind: "user"; text: string; /** When it was sent (ms since 1970), if known. */ at?: number }
  | { kind: "assistant"; text: string; streaming?: boolean }
  | { kind: "tool"; id: string; name: string; input: unknown; result?: string; isError?: boolean }
  | { kind: "permission"; id: string; toolName: string; input: Record<string, unknown>; state: "pending" | "allowed" | "denied" }
  | { kind: "done"; ms?: number; note?: string }
  | { kind: "error"; text: string };

export interface ClaudeStatus {
  installed: boolean;
  path: string | null;
  running: boolean;
  session: string | null;
  folder: string;
  /** Claude Code's own default model (~/.claude/settings.json), e.g. "opus". */
  defaultModel: string | null;
}

export interface PastChat {
  id: string;
  title: string;
  modified: number;
}

export interface AiContext {
  sessionId: string | null;
  connectionName: string | null;
  engine: string | null;
  database: string | null;
  openTable: string | null;
  editorSql: string | null;
}

interface State {
  items: ChatItem[];
  running: boolean;
  /** Index of the assistant item receiving streamed text. */
  streaming: number | null;
  /** Message ids whose text arrived by streaming (their final copy is skipped). */
  streamed: Set<string>;
  stoppedByUser: boolean;
  /** Claude changed data in this answer (execute_sql): refresh when it finishes. */
  wroteData: boolean;
  /** The model answering, as Claude Code reports it (e.g. "claude-opus-5-5"). */
  model: string | null;
}

let state: State = { items: [], running: false, streaming: null, streamed: new Set(), stoppedByUser: false, wroteData: false, model: null };
const listeners = new Set<() => void>();

function set(patch: Partial<State>) {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

export function useChat() {
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

function textOf(content: unknown): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((c) => (c && typeof c === "object" && (c as { type?: string }).type === "text" ? (c as { text: string }).text : ""))
      .join("");
  }
  return "";
}

/** Apply one Claude Code stream-JSON message (live, or replayed from a saved chat). */
export function applyEvent(ev: Record<string, unknown>, live: boolean) {
  const items = [...state.items];
  const type = ev.type as string;

  if (type === "system" && ev.subtype === "init" && typeof ev.model === "string") {
    set({ model: ev.model });
    return;
  }

  if (type === "stream_event") {
    const e = ev.event as Record<string, unknown>;
    if (e?.type === "message_start") {
      const id = (e.message as { id?: string })?.id;
      if (id) state.streamed.add(id);
    } else if (e?.type === "content_block_start" && (e.content_block as { type?: string })?.type === "text") {
      items.push({ kind: "assistant", text: "", streaming: true });
      set({ items, streaming: items.length - 1 });
      return;
    } else if (e?.type === "content_block_delta") {
      const delta = e.delta as { type?: string; text?: string };
      if (delta?.type === "text_delta" && state.streaming !== null) {
        const it = items[state.streaming];
        if (it?.kind === "assistant") items[state.streaming] = { ...it, text: it.text + (delta.text ?? "") };
        set({ items });
      }
      return;
    } else if (e?.type === "content_block_stop" && state.streaming !== null) {
      const it = items[state.streaming];
      if (it?.kind === "assistant") items[state.streaming] = { ...it, streaming: false };
      set({ items, streaming: null });
    }
    return;
  }

  if (type === "assistant") {
    const msg = ev.message as { id?: string; content?: unknown; model?: string };
    // Replayed chats have no init message: the model comes with each answer instead.
    const model = typeof msg?.model === "string" && !msg.model.startsWith("<") ? msg.model : state.model;
    const streamedAlready = live && msg?.id ? state.streamed.has(msg.id) : false;
    for (const block of Array.isArray(msg?.content) ? msg.content : []) {
      const b = block as { type: string; text?: string; id?: string; name?: string; input?: unknown };
      if (b.type === "text" && !streamedAlready && b.text?.trim()) items.push({ kind: "assistant", text: b.text });
      if (b.type === "tool_use") {
        items.push({ kind: "tool", id: b.id ?? "", name: (b.name ?? "").replace(/^mcp__tableboat__/, "tableboat › "), input: b.input });
      }
    }
    set({ items, model });
    return;
  }

  if (type === "user") {
    const msg = ev.message as { content?: unknown };
    const content = msg?.content;
    // Saved chats carry an ISO timestamp per message.
    const parsed = typeof ev.timestamp === "string" ? Date.parse(ev.timestamp) : NaN;
    const at = Number.isNaN(parsed) ? undefined : parsed;
    if (Array.isArray(content)) {
      for (const block of content) {
        const b = block as { type: string; tool_use_id?: string; content?: unknown; is_error?: boolean; text?: string };
        if (b.type === "tool_result") {
          const i = items.findIndex((x) => x.kind === "tool" && x.id === b.tool_use_id);
          if (i >= 0) {
            const tool = items[i] as Extract<ChatItem, { kind: "tool" }>;
            items[i] = { ...tool, result: textOf(b.content), isError: !!b.is_error };
            if (live && !b.is_error && tool.name.endsWith("execute_sql")) state.wroteData = true;
          }
        } else if (b.type === "text" && !live && b.text?.trim()) {
          items.push({ kind: "user", text: b.text, at });
        }
      }
    } else if (typeof content === "string" && !live && content.trim()) {
      items.push({ kind: "user", text: content, at });
    }
    set({ items });
    return;
  }

  if (type === "result") {
    const r = ev as { duration_ms?: number; is_error?: boolean; result?: string; subtype?: string };
    if (r.is_error) items.push({ kind: "error", text: r.result || `Claude Code stopped (${r.subtype ?? "error"}).` });
    items.push({ kind: "done", ms: r.duration_ms });
    const wrote = state.wroteData;
    set({ items, running: false, streaming: null, wroteData: false });
    // Claude changed data: reload what's on screen so it isn't stale.
    if (wrote) refreshAll();
  }
}

export function onPermission(p: { id: string; toolName: string; input: Record<string, unknown> }) {
  set({ items: [...state.items, { kind: "permission", ...p, state: "pending" }] });
}

export function onExit(stderr: string) {
  if (!state.running) return;
  const items = [...state.items];
  if (!state.stoppedByUser) {
    items.push({
      kind: "error",
      text: `Claude Code stopped unexpectedly.${stderr ? `\n${stderr.split("\n").slice(-6).join("\n")}` : ""}`,
    });
  }
  set({ items, running: false, streaming: null });
}

export async function send(text: string, context: AiContext) {
  set({ items: [...state.items, { kind: "user", text, at: Date.now() }], running: true, stoppedByUser: false });
  try {
    await invoke("claude_send", { text, context });
  } catch (err) {
    set({ items: [...state.items, { kind: "error", text: String(err) }], running: false });
  }
}

export async function stop() {
  set({ stoppedByUser: true });
  await invoke("claude_stop");
  if (state.running) set({ items: [...state.items, { kind: "done", note: "Stopped" }], running: false, streaming: null });
}

/** Stop the process when the pane closes (frees its memory); the chat resumes on the next message. */
export async function shutdown() {
  set({ stoppedByUser: true });
  await invoke("claude_stop");
  set({ running: false, streaming: null });
}

/** Forget the model Claude Code last reported (the model setting changed). */
export function clearModel() {
  if (state.model !== null) set({ model: null });
}

export async function newChat() {
  await invoke("claude_new", { resume: null });
  set({ items: [], running: false, streaming: null, streamed: new Set() });
}

export async function openPastChat(id: string) {
  const messages = await invoke<Record<string, unknown>[]>("claude_session_messages", { id });
  await invoke("claude_new", { resume: id });
  set({ items: [], running: false, streaming: null, streamed: new Set() });
  for (const m of messages) applyEvent(m, false);
  set({ items: [...state.items, { kind: "done", note: "Earlier chat – your next message continues it" }] });
}

export async function answerPermission(id: string, allow: boolean, always: boolean) {
  set({
    items: state.items.map((it) =>
      it.kind === "permission" && it.id === id ? { ...it, state: allow ? "allowed" : "denied" } : it,
    ),
  });
  await invoke("claude_permission_reply", { id, allow, always });
}

/**
 * A friendly model name: "claude-opus-5-5" -> "Opus 5.5", "claude-sonnet-4-5-20250929" -> "Sonnet 4.5",
 * "opus" -> "Opus", "opus[1m]" -> "Opus (1M)". Anything else is shown as given.
 */
export function modelLabel(id: string): string {
  const m = /^(?:claude-)?(opus|sonnet|haiku|fable)(?:-(\d+)(?:-(\d{1,2}))?)?(?:-\d{8})?(\[1m\])?$/i.exec(id.trim());
  if (!m) return id;
  const family = m[1][0].toUpperCase() + m[1].slice(1).toLowerCase();
  const version = m[2] ? ` ${m[2]}${m[3] ? `.${m[3]}` : ""}` : "";
  return `${family}${version}${m[4] ? " (1M)" : ""}`;
}

export const claudeApi = {
  status: () => invoke<ClaudeStatus>("claude_status"),
  sessions: () => invoke<PastChat[]>("claude_sessions"),
  setContext: (context: AiContext) => invoke<void>("claude_set_context", { context }),
};
