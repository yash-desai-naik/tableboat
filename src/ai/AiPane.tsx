import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { editors } from "../editor/registry";
import { openQuery, runAll } from "../editor/actions";
import {
  answerPermission,
  applyEvent,
  claudeApi,
  clearModel,
  modelLabel,
  newChat,
  onExit,
  onPermission,
  openPastChat,
  resetChat,
  send,
  stop,
  useChat,
  type AiContext,
  type ChatItem,
  type ClaudeStatus,
} from "../lib/claude";
import { displayName } from "../lib/connections";
import { getSession, useSession } from "../lib/session";
import { useSettings } from "../lib/settings";
import { activeTab, getWorkspace, useWorkspace } from "../lib/workspace";
import { CloseSmallIcon } from "../shell/Icons";
import { Menu, type MenuItem } from "../shell/Menu";

/** What Claude can see through get_context, read fresh from the app's state. */
export function currentContext(): AiContext {
  const session = getSession();
  const info = session.status === "open" ? session.info : null;
  const engine = info ? (info.server.engine ?? info.connection.engine) : null;
  const tab = activeTab();
  return {
    sessionId: info?.sessionId ?? null,
    connectionId: info?.connection.id ?? null,
    connectionName: info ? displayName(info.connection) : null,
    engine,
    database: engine === "sqlite" ? "main" : getWorkspace().database,
    openTable: tab?.type === "object" ? `${tab.database}.${tab.name}` : null,
    editorSql: tab?.type === "query" ? (editors.get(tab.id)?.text() ?? null) : null,
  };
}

let listening = false;

/** The Claude Code chat in the right pane (laid out like Claude Code's VS Code panel). */
export function AiPane({ onClose }: { onClose: () => void }) {
  const chat = useChat();
  const session = useSession();
  const ws = useWorkspace();
  const settings = useSettings();
  const [status, setStatus] = useState<ClaudeStatus | null>(null);
  // The model actually answering once Claude Code has started; before that, the one it will use.
  const model = chat.model ?? (settings.claudeModel.trim() || status?.defaultModel || null);
  const modelSetting = useRef(settings.claudeModel);
  useEffect(() => {
    if (modelSetting.current === settings.claudeModel) return;
    modelSetting.current = settings.claudeModel;
    clearModel();
  }, [settings.claudeModel]);
  const [draft, setDraft] = useState("");
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Keep Claude's view current as the connection, database or tab changes. A new connection
  // also means a new project folder, so the status (folder shown) is read again.
  const connectionId = session.status === "open" ? session.info.connection.id : null;
  useEffect(() => {
    void claudeApi.setContext(currentContext());
  }, [session, ws.database, ws.activeId]);
  useEffect(() => {
    void claudeApi
      .setContext(currentContext())
      .then(() => claudeApi.status())
      .then(setStatus);
  }, [connectionId]);

  useEffect(() => {
    void claudeApi.status().then(setStatus);
    if (listening) return;
    listening = true;
    const win = getCurrentWebviewWindow();
    void win.listen<Record<string, unknown>>("claude:event", (e) => applyEvent(e.payload, true));
    void win.listen<{ id: string; toolName: string; input: Record<string, unknown> }>("claude:permission", (e) => onPermission(e.payload));
    void win.listen<{ stderr: string }>("claude:exit", (e) => onExit(e.payload.stderr));
    // Another connection in this window: its own Claude Code project, so a fresh chat.
    void win.listen("claude:reset", () => resetChat());
  }, []);

  // Follow new output.
  useLayoutEffect(() => {
    const el = listRef.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 160) el.scrollTop = el.scrollHeight;
  }, [chat.items, chat.running]);

  // Ctrl+Esc moves focus into the chat input, or back to the editor.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || !e.ctrlKey || e.shiftKey || e.altKey) return;
      e.preventDefault();
      const input = inputRef.current;
      if (!input) return;
      if (document.activeElement === input) {
        input.blur();
        const tab = activeTab();
        if (tab?.type === "query") editors.get(tab.id)?.focus();
      } else {
        input.focus();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  // The input grows with its text, up to a limit.
  useLayoutEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 220)}px`;
  }, [draft]);

  function submit() {
    const text = draft.trim();
    if (!text || chat.running) return;
    setDraft("");
    void send(text, currentContext());
  }

  async function pastChats(x: number, y: number) {
    const list = await claudeApi.sessions();
    setMenu({
      x,
      y,
      items: list.length
        ? list.slice(0, 25).map((c) => ({
            label: c.title.length > 60 ? `${c.title.slice(0, 60)}…` : c.title,
            hint: new Date(c.modified * 1000).toLocaleDateString(),
            onSelect: () => void openPastChat(c.id),
          }))
        : [{ label: "No earlier chats yet", disabled: true, onSelect: () => {} }],
    });
  }

  const firstUser = chat.items.find((i) => i.kind === "user");
  const title = firstUser?.kind === "user" ? firstUser.text.replace(/\s+/g, " ") : "New chat";
  const pending = chat.items.some((i) => i.kind === "permission" && i.state === "pending");
  const sees = session.status === "open" ? `${displayName(session.info.connection)}${ws.database ? ` · ${ws.database}` : ""}` : null;

  return (
    <div
      className="ai"
      onKeyDown={(e) => {
        // Esc stops Claude (unless a permission prompt is waiting: that handles its own keys).
        if (e.key === "Escape" && chat.running && !pending) {
          e.preventDefault();
          void stop();
        }
      }}
    >
      <div className="ai-head">
        <ClaudeMark />
        <span className="ai-chat-title" title={title}>
          {title}
        </span>
        <button
          className="icon-btn small"
          title="Earlier chats"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            void pastChats(r.right - 320, r.bottom + 4);
          }}
        >
          <HistoryIcon />
        </button>
        <button className="icon-btn small" title="New chat" onClick={() => void newChat()}>
          <NewChatIcon />
        </button>
        <button className="icon-btn small" title="Close (Ctrl+Alt+B)" onClick={onClose}>
          <CloseSmallIcon />
        </button>
      </div>

      <div className="ai-list" ref={listRef}>
        {status && !status.installed && (
          <div className="ai-note error">
            Claude Code (claude.exe) wasn't found. Install it and sign in once in a terminal, then reopen this pane.
          </div>
        )}
        {chat.items.length === 0 && status?.installed && (
          <div className="ai-empty">
            <ClaudeMark size={30} />
            <p className="ai-empty-title">Ask about your database, or have Claude write SQL.</p>
            <p className="ai-empty-note">
              {session.status === "open" ? "This connection's" : "General"} Claude Code project:{" "}
              <code>{status.folder}</code> (chats and the CLAUDE.md notes there are kept for this connection). It can read
              your schema and run read-only queries; anything else asks you first.
            </p>
            <div className="ai-suggestions">
              {["What tables are in this database?", "Explain the SQL in my editor", "Write a query for the open table's most recent rows"].map((s) => (
                <button key={s} className="ai-suggestion" onClick={() => void send(s, currentContext())}>
                  {s}
                </button>
              ))}
            </div>
          </div>
        )}
        {chat.items.map((item, i) => (
          <Item key={i} item={item} />
        ))}
        {chat.running && !pending && <Working />}
      </div>

      <div className="ai-compose">
        <div className="ai-composer" onClick={() => inputRef.current?.focus()}>
          <textarea
            ref={inputRef}
            className="ai-input"
            rows={1}
            placeholder={status?.installed === false ? "Claude Code isn't installed" : "Ctrl+Esc to focus or unfocus Claude"}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                submit();
              }
            }}
            disabled={status?.installed === false}
          />
          <div className="ai-composer-bar">
            <span className="ai-chip" title={sees ? `Claude sees ${sees}` : "Not connected to a database"}>
              <span className={sees ? "ai-chip-dot on" : "ai-chip-dot"} />
              <span className="ai-chip-text">{sees ?? "Not connected"}</span>
            </span>
            {status?.installed && (
              <span
                className="ai-chip ai-model"
                title={model ? `Model: ${model}${chat.model ? "" : " (used from your next message)"}\nChange it in Settings › Claude model` : "Claude Code's default model\nChange it in Settings › Claude model"}
              >
                {model ? modelLabel(model) : "Default model"}
              </span>
            )}
            <span className="spacer" />
            <span className="ai-mode" title="Claude reads your schema and runs read-only queries freely; anything that changes data asks you first">
              <ShieldIcon />
              <span className="ai-mode-text">Asks before writes</span>
            </span>
            {chat.running ? (
              <button
                className="ai-send stop"
                title="Stop (Esc)"
                onClick={(e) => {
                  e.stopPropagation();
                  void stop();
                }}
              >
                <StopIcon />
              </button>
            ) : (
              <button
                className="ai-send"
                title="Send (Enter)"
                disabled={!draft.trim()}
                onClick={(e) => {
                  e.stopPropagation();
                  submit();
                }}
              >
                <ArrowUpIcon />
              </button>
            )}
          </div>
        </div>
      </div>
      {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}

const WORKING_WORDS = ["Thinking", "Reading the schema", "Querying", "Working"];

/** The live "Claude is working" row: spinning mark, elapsed time, how to stop. */
function Working() {
  const [started] = useState(() => Date.now());
  const [now, setNow] = useState(started);
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, []);
  const secs = Math.floor((now - started) / 1000);
  return (
    <div className="ai-working">
      <ClaudeMark spin />
      <span className="ai-working-word">{WORKING_WORDS[Math.floor(secs / 6) % WORKING_WORDS.length]}…</span>
      <span className="muted">
        {secs}s · Esc to stop
      </span>
    </div>
  );
}

/** Friendly names for Tableboat's own tools. */
const TOOL_NAMES: Record<string, string> = {
  get_context: "Context",
  get_schema: "Schema",
  run_read_query: "Read query",
  execute_sql: "Execute SQL",
};

function toolTitle(name: string): string {
  const own = /^tableboat › (.+)$/.exec(name) ?? /^mcp__tableboat__(.+)$/.exec(name);
  return own ? (TOOL_NAMES[own[1]] ?? own[1]) : name;
}

/** One timeline step: a dot in the left gutter, then the content. */
function Step({ dot, children }: { dot: "text" | "ok" | "error" | "running"; children: ReactNode }) {
  return (
    <div className="ai-step">
      <span className={`ai-dot ${dot}`} />
      <div className="ai-step-body">{children}</div>
    </div>
  );
}

function Item({ item }: { item: ChatItem }) {
  const [open, setOpen] = useState(false);
  switch (item.kind) {
    case "user":
      return (
        <div className="ai-user-wrap">
          {item.at !== undefined && (
            <div className="ai-time" title={new Date(item.at).toLocaleString()}>
              {new Date(item.at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}
            </div>
          )}
          <div className="ai-user">{item.text}</div>
        </div>
      );
    case "assistant":
      return (
        <Step dot="text">
          <div className="ai-msg">
            <Markdown text={item.text} />
            {item.streaming && <span className="ai-caret" />}
          </div>
        </Step>
      );
    case "tool": {
      // Claude Code's own tool lookup is internal noise.
      if (item.name === "ToolSearch") return null;
      const summary = toolSummary(item.input);
      const dot = item.result === undefined ? "running" : item.isError ? "error" : "ok";
      return (
        <Step dot={dot}>
          <button className="ai-tool-head" onClick={() => setOpen((o) => !o)} title={open ? "Hide details" : "Show details"}>
            <span className="ai-tool-name">{toolTitle(item.name)}</span>
            {summary && <span className="ai-tool-sum">{summary}</span>}
          </button>
          {open && (
            <div className="ai-io">
              <div className="ai-io-row">
                <span className="ai-io-label">IN</span>
                <pre>{toolInput(item.input)}</pre>
              </div>
              {item.result !== undefined && (
                <div className="ai-io-row">
                  <span className="ai-io-label">OUT</span>
                  <pre className={item.isError ? "error" : undefined}>{item.result.slice(0, 6000) || "(no output)"}</pre>
                </div>
              )}
            </div>
          )}
        </Step>
      );
    }
    case "permission":
      return <Permission item={item} />;
    case "done":
      return item.note ? (
        <div className="ai-note-line">{item.note}</div>
      ) : (
        <div className="ai-done">{item.ms !== undefined ? `${(item.ms / 1000).toFixed(1)} s` : ""}</div>
      );
    case "error":
      return (
        <Step dot="error">
          <div className="ai-error-text">{item.text}</div>
        </Step>
      );
  }
}

function Permission({ item }: { item: Extract<ChatItem, { kind: "permission" }> }) {
  const name = toolTitle(item.toolName.replace(/^mcp__tableboat__/, "tableboat › "));
  const detail =
    (item.input.sql as string) ?? (item.input.command as string) ?? (item.input.file_path as string) ?? JSON.stringify(item.input, null, 2);
  const first = useRef<HTMLButtonElement>(null);
  const pending = item.state === "pending";
  useEffect(() => {
    if (pending) first.current?.focus();
  }, [pending]);
  const choose = (allow: boolean, always: boolean) => void answerPermission(item.id, allow, always);
  const question = /sql/i.test(item.toolName) ? "Run this SQL?" : `Allow Claude to use ${name}?`;
  return (
    <div
      className={`ai-perm ${item.state}`}
      onKeyDown={(e) => {
        if (!pending) return;
        const k = e.key;
        if (k === "1" || k === "2" || k === "3" || k === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          if (k === "1") choose(true, false);
          else if (k === "2") choose(true, true);
          else choose(false, false);
        }
      }}
    >
      <div className="ai-perm-title">{pending ? question : item.state === "allowed" ? `Allowed: ${name}` : `Denied: ${name}`}</div>
      <pre className="ai-perm-detail">{String(detail).slice(0, 4000)}</pre>
      {pending && (
        <div className="ai-perm-options">
          <button ref={first} className="ai-perm-option" onClick={() => choose(true, false)}>
            <kbd>1</kbd> Yes
          </button>
          <button className="ai-perm-option" onClick={() => choose(true, true)}>
            <kbd>2</kbd> Yes, and don't ask again for {name} in this chat
          </button>
          <button className="ai-perm-option" onClick={() => choose(false, false)}>
            <kbd>3</kbd> No <span className="muted">(Esc)</span>
          </button>
        </div>
      )}
    </div>
  );
}

/** A small starburst in Claude's clay colour, drawn here (not Anthropic's logo file). */
function ClaudeMark({ size = 14, spin = false }: { size?: number; spin?: boolean }) {
  return (
    <svg className={spin ? "ai-mark spin" : "ai-mark"} width={size} height={size} viewBox="0 0 16 16" aria-hidden="true">
      {[0, 30, 60, 90, 120, 150].map((a) => (
        <rect key={a} x="7.1" y="1" width="1.8" height="14" rx="0.9" transform={`rotate(${a} 8 8)`} />
      ))}
    </svg>
  );
}

const HistoryIcon = () => (
  <svg width="15" height="15" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round">
    <circle cx="8" cy="8" r="6" />
    <path d="M8 4.8V8l2.2 1.6" />
  </svg>
);
const NewChatIcon = () => (
  <svg width="15" height="15" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round">
    <path d="M8 2.2a5.8 5.8 0 1 0 2.7 10.9L13.8 14l-.8-3A5.8 5.8 0 0 0 8 2.2Z" />
    <path d="M8 5.6v4.8M5.6 8h4.8" />
  </svg>
);
const ShieldIcon = () => (
  <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round">
    <path d="M8 1.8 2.8 3.8v4c0 3.2 2.2 5.4 5.2 6.4 3-1 5.2-3.2 5.2-6.4v-4Z" />
  </svg>
);
const ArrowUpIcon = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
    <path d="M7 11.5V2.8M3 6.6 7 2.6l4 4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);
const StopIcon = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
    <rect x="1" y="1" width="8" height="8" rx="1.5" fill="currentColor" />
  </svg>
);

/** A tool's input for the IN box: SQL or a command as is, anything else as JSON. */
function toolInput(input: unknown): string {
  if (!input || typeof input !== "object") return String(input ?? "");
  const i = input as Record<string, unknown>;
  for (const k of ["sql", "query", "command"]) if (typeof i[k] === "string") return i[k] as string;
  return JSON.stringify(input, null, 2);
}

function toolSummary(input: unknown): string {
  if (!input || typeof input !== "object") return "";
  const i = input as Record<string, unknown>;
  const s = (i.sql ?? i.table ?? i.command ?? i.file_path ?? i.pattern ?? i.query ?? "") as string;
  return typeof s === "string" ? s.replace(/\s+/g, " ").slice(0, 80) : "";
}

/** Insert SQL into the active editor tab, or a new one. */
function insertSql(sql: string) {
  const tab = activeTab();
  const ed = tab?.type === "query" ? editors.get(tab.id) : undefined;
  if (ed) ed.insert(sql);
  else openQuery(sql);
}

/** Open SQL in a new tab and run it there. */
function runSql(sql: string) {
  openQuery(sql);
  // The editor mounts after the tab opens (and may still be loading its code).
  const started = Date.now();
  const tick = () => {
    const tab = activeTab();
    if (tab?.type === "query" && editors.has(tab.id)) runAll();
    else if (Date.now() - started < 4000) window.setTimeout(tick, 50);
  };
  tick();
}

/**
 * A small, safe Markdown renderer: code fences (with Insert / Run for SQL), inline code,
 * bold, and line breaks. Text is never injected as HTML.
 */
function Markdown({ text }: { text: string }) {
  const parts: ReactNode[] = [];
  const fence = /```(\w*)\n?([\s\S]*?)(```|$)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let key = 0;
  while ((m = fence.exec(text))) {
    if (m.index > last) parts.push(<Prose key={key++} text={text.slice(last, m.index)} />);
    const lang = m[1].toLowerCase();
    const code = m[2].replace(/\n$/, "");
    const isSql = lang === "sql" || lang === "mysql" || lang === "sqlite" || lang === "";
    parts.push(
      <div key={key++} className="ai-code">
        <div className="ai-code-bar">
          <span>{lang || "code"}</span>
          <span className="spacer" />
          <button className="link-btn" onClick={() => void navigator.clipboard.writeText(code)}>
            Copy
          </button>
          {isSql && (
            <>
              <button className="link-btn" onClick={() => insertSql(code)}>
                Insert
              </button>
              <button className="link-btn" onClick={() => runSql(code)}>
                Run
              </button>
            </>
          )}
        </div>
        <pre>{code}</pre>
      </div>,
    );
    last = fence.lastIndex;
    if (m[3] === "") break;
  }
  if (last < text.length) parts.push(<Prose key={key++} text={text.slice(last)} />);
  return <>{parts}</>;
}

function Prose({ text }: { text: string }) {
  const lines = text.replace(/^\n+|\n+$/g, "").split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    // A Markdown table: consecutive lines starting with "|".
    if (line.trim().startsWith("|")) {
      const rows: string[][] = [];
      while (i < lines.length && lines[i].trim().startsWith("|")) {
        const cells = lines[i].trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => c.trim());
        if (!cells.every((c) => /^:?-{2,}:?$/.test(c))) rows.push(cells);
        i++;
      }
      const [head, ...body] = rows;
      blocks.push(
        <table key={blocks.length} className="ai-table">
          {head && (
            <thead>
              <tr>{head.map((c, k) => <th key={k}>{inline(c)}</th>)}</tr>
            </thead>
          )}
          <tbody>
            {body.map((r, n) => (
              <tr key={n}>{r.map((c, k) => <td key={k}>{inline(c)}</td>)}</tr>
            ))}
          </tbody>
        </table>,
      );
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    const bullet = /^(\s*)[-*]\s+(.*)$/.exec(line);
    const numbered = /^(\s*)(\d+)\.\s+(.*)$/.exec(line);
    if (heading) blocks.push(<div key={blocks.length} className="ai-h">{inline(heading[2])}</div>);
    else if (bullet)
      blocks.push(
        <div key={blocks.length} className="ai-li" style={{ paddingLeft: 4 + bullet[1].length * 8 }}>
          <span className="ai-li-mark">•</span>
          <span>{inline(bullet[2])}</span>
        </div>,
      );
    else if (numbered)
      blocks.push(
        <div key={blocks.length} className="ai-li" style={{ paddingLeft: 4 + numbered[1].length * 8 }}>
          <span className="ai-li-mark">{numbered[2]}.</span>
          <span>{inline(numbered[3])}</span>
        </div>,
      );
    else if (line.trim() === "") blocks.push(<div key={blocks.length} className="ai-gap" />);
    else blocks.push(<div key={blocks.length}>{inline(line)}</div>);
    i++;
  }
  return <div className="ai-prose">{blocks}</div>;
}

function inline(line: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(\*\*[^*]+\*\*|`[^`]+`)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let k = 0;
  while ((m = re.exec(line))) {
    if (m.index > last) out.push(line.slice(last, m.index));
    const tok = m[0];
    out.push(tok.startsWith("`") ? <code key={k++}>{tok.slice(1, -1)}</code> : <strong key={k++}>{inline(tok.slice(2, -2))}</strong>);
    last = re.lastIndex;
  }
  if (last < line.length) out.push(line.slice(last));
  return out;
}
