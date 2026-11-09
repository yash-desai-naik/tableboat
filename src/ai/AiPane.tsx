import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { editors } from "../editor/registry";
import { openQuery, runAll } from "../editor/actions";
import {
  answerPermission,
  applyEvent,
  claudeApi,
  newChat,
  onExit,
  onPermission,
  openPastChat,
  send,
  stop,
  useChat,
  type AiContext,
  type ChatItem,
  type ClaudeStatus,
} from "../lib/claude";
import { displayName } from "../lib/connections";
import { getSession, useSession } from "../lib/session";
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
    connectionName: info ? displayName(info.connection) : null,
    engine,
    database: engine === "sqlite" ? "main" : getWorkspace().database,
    openTable: tab?.type === "object" ? `${tab.database}.${tab.name}` : null,
    editorSql: tab?.type === "query" ? (editors.get(tab.id)?.text() ?? null) : null,
  };
}

let listening = false;

/** The Claude Code chat in the right pane. */
export function AiPane({ onClose }: { onClose: () => void }) {
  const chat = useChat();
  const session = useSession();
  const ws = useWorkspace();
  const [status, setStatus] = useState<ClaudeStatus | null>(null);
  const [draft, setDraft] = useState("");
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Keep Claude's view current as the connection, database or tab changes.
  useEffect(() => {
    void claudeApi.setContext(currentContext());
  }, [session, ws.database, ws.activeId]);

  useEffect(() => {
    void claudeApi.status().then(setStatus);
    if (listening) return;
    listening = true;
    const win = getCurrentWebviewWindow();
    void win.listen<Record<string, unknown>>("claude:event", (e) => applyEvent(e.payload, true));
    void win.listen<{ id: string; toolName: string; input: Record<string, unknown> }>("claude:permission", (e) => onPermission(e.payload));
    void win.listen<{ stderr: string }>("claude:exit", (e) => onExit(e.payload.stderr));
  }, []);

  // Follow new output.
  useLayoutEffect(() => {
    const el = listRef.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 160) el.scrollTop = el.scrollHeight;
  }, [chat.items]);

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

  return (
    <div className="ai">
      <div className="pane-header">
        <span className="pane-title">Claude</span>
        <span className="ai-head-actions">
          <button
            className="icon-btn small"
            title="Earlier chats"
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              void pastChats(r.left - 180, r.bottom + 4);
            }}
          >
            ⟲
          </button>
          <button className="icon-btn small" title="New chat" onClick={() => void newChat()}>
            +
          </button>
          <button className="icon-btn small" title="Close (Ctrl+Alt+B)" onClick={onClose}>
            <CloseSmallIcon />
          </button>
        </span>
      </div>

      <div className="ai-list" ref={listRef}>
        {status && !status.installed && (
          <div className="ai-note error">
            Claude Code (claude.exe) wasn't found. Install it and sign in once in a terminal, then reopen this pane.
          </div>
        )}
        {chat.items.length === 0 && status?.installed && (
          <div className="ai-empty">
            <p>Ask about your database, or have Claude write SQL.</p>
            <p className="muted small">
              Claude Code runs in <code>{status.folder}</code>. It can read your schema and run read-only queries; anything
              else asks you first.
            </p>
            <div className="ai-suggestions">
              {["What tables are in this database?", "Explain the SQL in my editor", "Write a query for the open table's most recent rows"].map((s) => (
                <button key={s} className="btn small-btn" onClick={() => void send(s, currentContext())}>
                  {s}
                </button>
              ))}
            </div>
          </div>
        )}
        {chat.items.map((item, i) => (
          <Item key={i} item={item} />
        ))}
        {chat.running && <div className="ai-working">Claude is working…</div>}
      </div>

      <div className="ai-compose">
        <textarea
          ref={inputRef}
          className="input ai-input"
          rows={3}
          placeholder={session.status === "open" ? "Ask Claude…  (Enter to send, Shift+Enter for a new line)" : "Ask Claude…"}
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
        <div className="ai-compose-bar">
          <span className="muted small">{session.status === "open" ? `Sees: ${displayName(session.info.connection)}${ws.database ? ` · ${ws.database}` : ""}` : "Not connected"}</span>
          <span className="spacer" />
          {chat.running ? (
            <button className="btn small-btn" onClick={() => void stop()}>
              Stop
            </button>
          ) : (
            <button className="btn primary small-btn" onClick={submit} disabled={!draft.trim()}>
              Send
            </button>
          )}
        </div>
      </div>
      {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}

function Item({ item }: { item: ChatItem }) {
  const [open, setOpen] = useState(false);
  switch (item.kind) {
    case "user":
      return <div className="ai-msg user">{item.text}</div>;
    case "assistant":
      return (
        <div className="ai-msg assistant">
          <Markdown text={item.text} />
          {item.streaming && <span className="ai-caret" />}
        </div>
      );
    case "tool": {
      // Claude Code's own tool lookup is internal noise.
      if (item.name === "ToolSearch") return null;
      const summary = toolSummary(item.input);
      return (
        <div className={item.isError ? "ai-tool error" : "ai-tool"}>
          <button className="ai-tool-head" onClick={() => setOpen((o) => !o)}>
            <span>{item.result === undefined ? "◌" : item.isError ? "✕" : "✓"}</span>
            <span className="ai-tool-name">{item.name}</span>
            {summary && <span className="ai-tool-sum">{summary}</span>}
          </button>
          {open && item.result !== undefined && <pre className="ai-tool-result">{item.result.slice(0, 6000)}</pre>}
        </div>
      );
    }
    case "permission":
      return <Permission item={item} />;
    case "done":
      return (
        <div className="ai-done">
          {item.note ??
            [item.ms !== undefined ? `${(item.ms / 1000).toFixed(1)} s` : null, item.cost ? `$${item.cost.toFixed(4)}` : null]
              .filter(Boolean)
              .join(" · ")}
        </div>
      );
    case "error":
      return <div className="ai-note error">{item.text}</div>;
  }
}

function Permission({ item }: { item: Extract<ChatItem, { kind: "permission" }> }) {
  const name = item.toolName.replace(/^mcp__tableboat__/, "tableboat › ");
  const detail =
    (item.input.sql as string) ?? (item.input.command as string) ?? (item.input.file_path as string) ?? JSON.stringify(item.input, null, 2);
  return (
    <div className={`ai-perm ${item.state}`}>
      <div className="ai-perm-title">
        {item.state === "pending" ? "Claude wants to use" : item.state === "allowed" ? "Allowed" : "Denied"} <b>{name}</b>
      </div>
      <pre className="ai-perm-detail">{String(detail).slice(0, 4000)}</pre>
      {item.state === "pending" && (
        <div className="ai-perm-actions">
          <button className="btn small-btn" onClick={() => void answerPermission(item.id, false, false)}>
            Deny
          </button>
          <button className="btn small-btn" onClick={() => void answerPermission(item.id, true, true)} title="Don't ask again for this tool in this chat">
            Always allow
          </button>
          <button className="btn primary small-btn" onClick={() => void answerPermission(item.id, true, false)}>
            Allow
          </button>
        </div>
      )}
    </div>
  );
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
    else if (bullet) blocks.push(<div key={blocks.length} className="ai-li" style={{ paddingLeft: 12 + bullet[1].length * 6 }}>• {inline(bullet[2])}</div>);
    else if (numbered) blocks.push(<div key={blocks.length} className="ai-li" style={{ paddingLeft: 12 + numbered[1].length * 6 }}>{numbered[2]}. {inline(numbered[3])}</div>);
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
