import { useEffect, useMemo, useRef, useState } from "react";
import { ask, message, open as openFile, save as saveFile } from "@tauri-apps/plugin-dialog";
import {
  api,
  describeTarget,
  displayName,
  engineInfo,
  errorText,
  newConnection,
  tagInfo,
  type Connection,
  type ConnectionSummary,
  type Secrets,
} from "../lib/connections";
import { connect, useSession } from "../lib/session";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { lastConnection } from "../lib/persist";
import { getSettings } from "../lib/settings";
import { Dialog } from "../shell/Dialog";
import { Menu, type MenuItem } from "../shell/Menu";
import { ConnectionForm } from "./ConnectionForm";

type FormState =
  | null
  | { mode: "new"; initial: Connection; secrets?: Secrets }
  | { mode: "edit"; summary: ConnectionSummary };

/** Imperative entry points used by commands in the palette. */
export const connectionActions = {
  newConnection: () => {},
  importUrl: () => {},
  importFile: () => {},
  exportFile: () => {},
};

let startupChecked = false;

/** The last connection, once per app start, for the main window (when the setting is on). */
async function startupConnection(): Promise<string | null> {
  if (startupChecked) return null;
  startupChecked = true;
  if (getCurrentWindow().label !== "main" || !getSettings().reopenLastConnection) return null;
  const id = await lastConnection();
  if (!id) return null;
  const c = (await api.list()).find((x) => x.id === id);
  // Never at start-up for "ask every time" connections: no surprise password prompt.
  return c && (c.savePassword || c.engine === "sqlite") ? id : null;
}

export function ConnectionList() {
  const session = useSession();
  const [items, setItems] = useState<ConnectionSummary[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [askPassword, setAskPassword] = useState<ConnectionSummary | null>(null);
  const [urlDialog, setUrlDialog] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);

  async function reload() {
    try {
      setItems(await api.list());
    } catch (err) {
      setError(errorText(err));
    } finally {
      setLoaded(true);
    }
  }

  useEffect(() => {
    void (async () => {
      await reload();
      // A window opened with "Connect in New Window" connects straight away (once).
      const params = new URLSearchParams(window.location.search);
      const id = params.get("connect") ?? (await startupConnection());
      if (!id) return;
      window.history.replaceState(null, "", window.location.pathname);
      const target = (await api.list()).find((c) => c.id === id);
      if (target) await open(target);
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const sorted = [...items].sort((a, b) => displayName(a).localeCompare(displayName(b)));
    if (!q) return sorted;
    return sorted.filter((c) =>
      [displayName(c), describeTarget(c), c.tag ?? "", engineInfo(c.engine).label].some((s) =>
        s.toLowerCase().includes(q),
      ),
    );
  }, [items, query]);

  const connecting = session.status === "connecting" ? session.connectionId : null;

  async function open(c: ConnectionSummary, password?: string) {
    setError(null);
    try {
      await connect(c.id, displayName(c), password);
    } catch (err) {
      const text = errorText(err);
      if (text === "PASSWORD_REQUIRED") setAskPassword(c);
      else setError(`${displayName(c)}: ${text}`);
    }
  }

  async function remove(c: ConnectionSummary) {
    const ok = await ask(`Delete the connection "${displayName(c)}"? Its saved passwords are removed too.`, {
      title: "Delete connection",
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    try {
      await api.remove(c.id);
      await reload();
    } catch (err) {
      setError(errorText(err));
    }
  }

  async function copyUrl(c: ConnectionSummary, withPasswords: boolean) {
    try {
      await navigator.clipboard.writeText(await api.toUrl(c.id, withPasswords));
    } catch (err) {
      setError(errorText(err));
    }
  }

  async function importFile() {
    const path = await openFile({
      title: "Import connections",
      multiple: false,
      directory: false,
      filters: [
        { name: "Connection list", extensions: ["txt", "urls"] },
        { name: "All files", extensions: ["*"] },
      ],
    });
    if (typeof path !== "string") return;
    try {
      const result = await api.importFile(path);
      await reload();
      const problems = result.errors.length ? `\n\nSkipped:\n${result.errors.join("\n")}` : "";
      await message(`Imported ${result.imported} connection(s).${problems}`, { title: "Import connections" });
    } catch (err) {
      setError(errorText(err));
    }
  }

  async function exportFile() {
    const includePasswords = await ask(
      "Include passwords in the exported file? Anyone who gets the file can then read them.",
      { title: "Export connections", kind: "warning", okLabel: "Include passwords", cancelLabel: "Without passwords" },
    );
    const path = await saveFile({
      title: "Export connections",
      defaultPath: "tableboat-connections.txt",
      filters: [{ name: "Connection list", extensions: ["txt"] }],
    });
    if (!path) return;
    try {
      const count = await api.exportFile(path, includePasswords);
      await message(`Exported ${count} connection(s) to ${path}.`, { title: "Export connections" });
    } catch (err) {
      setError(errorText(err));
    }
  }

  connectionActions.newConnection = () => setForm({ mode: "new", initial: newConnection() });
  connectionActions.importUrl = () => setUrlDialog(true);
  connectionActions.importFile = () => void importFile();
  connectionActions.exportFile = () => void exportFile();

  function contextItems(c: ConnectionSummary): MenuItem[] {
    return [
      { label: "Connect", hint: "Enter", onSelect: () => void open(c) },
      { label: "Connect in New Window", onSelect: () => void api.newWindow(c.id) },
      "separator",
      { label: "Edit…", onSelect: () => setForm({ mode: "edit", summary: c }) },
      {
        label: "Duplicate",
        onSelect: async () => {
          try {
            const copy = await api.duplicate(c.id);
            await reload();
            setSelected(copy.id);
          } catch (err) {
            setError(errorText(err));
          }
        },
      },
      "separator",
      { label: "Copy as URL", onSelect: () => void copyUrl(c, false) },
      { label: "Copy as URL with Passwords", onSelect: () => void copyUrl(c, true) },
      "separator",
      { label: "Delete…", danger: true, onSelect: () => void remove(c) },
    ];
  }

  function onListKey(e: React.KeyboardEvent) {
    if (!filtered.length) return;
    const index = filtered.findIndex((c) => c.id === selected);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = e.key === "ArrowDown" ? Math.min(filtered.length - 1, index + 1) : Math.max(0, index - 1);
      setSelected(filtered[next].id);
      document.getElementById(`conn-${filtered[next].id}`)?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      const c = filtered[index] ?? filtered[0];
      if (c) {
        e.preventDefault();
        void open(c);
      }
    } else if (e.key === "Delete") {
      const c = filtered[index];
      if (c) void remove(c);
    }
  }

  return (
    <div className="conn-screen">
      <div className="conn-panel">
        <div className="conn-toolbar">
          <input
            ref={searchRef}
            className="input"
            placeholder="Search connections"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setSelected(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown" || e.key === "Enter" || e.key === "ArrowUp") onListKey(e);
              if (e.key === "Escape" && query) {
                e.preventDefault();
                setQuery("");
              }
            }}
            spellCheck={false}
            autoFocus
          />
          <button className="btn primary" onClick={() => setForm({ mode: "new", initial: newConnection() })}>
            New
          </button>
          <button className="btn" onClick={() => setUrlDialog(true)}>
            From URL
          </button>
          <button
            className="btn"
            aria-label="More"
            title="Import and export"
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              setMenu({
                x: r.left,
                y: r.bottom + 4,
                items: [
                  { label: "Import from File…", onSelect: () => void importFile() },
                  { label: "Export to File…", onSelect: () => void exportFile() },
                ],
              });
            }}
          >
            ⋯
          </button>
        </div>

        {error && (
          <div className="notice error" role="alert">
            {error}
            <button className="link-btn" onClick={() => setError(null)}>
              Dismiss
            </button>
          </div>
        )}

        <div className="conn-list" role="listbox" aria-label="Saved connections" tabIndex={0} onKeyDown={onListKey}>
          {loaded && items.length === 0 && (
            <div className="conn-empty">
              <p>No saved connections yet.</p>
              <p className="muted">Create one, or paste a connection URL.</p>
            </div>
          )}
          {loaded && items.length > 0 && filtered.length === 0 && (
            <div className="conn-empty muted">No connection matches “{query}”.</div>
          )}
          {filtered.map((c) => {
            const tag = tagInfo(c.tag);
            const isBusy = connecting === c.id;
            return (
              <div
                key={c.id}
                id={`conn-${c.id}`}
                role="option"
                aria-selected={selected === c.id}
                className={selected === c.id ? "conn-row selected" : "conn-row"}
                onClick={() => setSelected(c.id)}
                onDoubleClick={() => void open(c)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setSelected(c.id);
                  setMenu({ x: e.clientX, y: e.clientY, items: contextItems(c) });
                }}
              >
                <span className="conn-badge" style={{ background: c.color }}>
                  {engineInfo(c.engine).badge}
                </span>
                <span className="conn-text">
                  <span className="conn-name">{displayName(c)}</span>
                  <span className="conn-target">{describeTarget(c)}</span>
                </span>
                {c.ssh.enabled && <span className="conn-flag">SSH</span>}
                {tag && (
                  <span className="tag-chip" style={{ ["--tag" as string]: tag.color }}>
                    {tag.short}
                  </span>
                )}
                {isBusy && <span className="conn-busy">Connecting…</span>}
              </div>
            );
          })}
        </div>
        <div className="conn-hint muted">
          Double-click or Enter to connect · right-click for more · passwords are kept in Windows Credential Manager
        </div>
      </div>

      {form && (
        <ConnectionForm
          title={form.mode === "new" ? "New connection" : `Edit ${displayName(form.summary)}`}
          initial={form.mode === "new" ? form.initial : form.summary}
          initialSecrets={form.mode === "new" ? form.secrets : undefined}
          stored={form.mode === "edit" ? form.summary : undefined}
          onClose={() => setForm(null)}
          onSaved={async (saved, connectWith) => {
            setForm(null);
            await reload();
            setSelected(saved.id);
            if (connectWith) await open(saved, connectWith.password);
          }}
        />
      )}

      {urlDialog && (
        <UrlDialog
          onClose={() => setUrlDialog(false)}
          onParsed={(connection, secrets) => {
            setUrlDialog(false);
            setForm({ mode: "new", initial: connection, secrets });
          }}
        />
      )}

      {askPassword && (
        <PasswordDialog
          name={displayName(askPassword)}
          onClose={() => setAskPassword(null)}
          onSubmit={(pw) => {
            const c = askPassword;
            setAskPassword(null);
            void open(c, pw);
          }}
        />
      )}

      {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}

export function UrlDialog({
  onClose,
  onParsed,
}: {
  onClose: () => void;
  onParsed: (c: Connection, s: Secrets) => void;
}) {
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    try {
      const parsed = await api.parseUrl(text);
      onParsed(parsed.connection, parsed.secrets);
    } catch (err) {
      setError(errorText(err));
    }
  }

  return (
    <Dialog
      title="New connection from URL"
      onClose={onClose}
      footer={
        <>
          <span className="spacer" />
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" onClick={submit} disabled={!text.trim()}>
            Continue
          </button>
        </>
      }
    >
      <textarea
        className="input mono url-box"
        placeholder={"mysql://user:password@host:3306/database\nmysql+ssh://sshuser@bastion:22/user:password@10.0.0.5:3306/db\nsqlite:///C:/path/to/file.db"}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          setError(null);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            void submit();
          }
        }}
        spellCheck={false}
      />
      <p className="muted small">
        You can check and change everything on the next screen before saving. Options such as{" "}
        <code>name</code>, <code>color</code>, <code>tag</code> and <code>ssl-mode</code> are read from the URL.
      </p>
      {error && <div className="notice error">{error}</div>}
    </Dialog>
  );
}

export function PasswordDialog({
  name,
  onClose,
  onSubmit,
}: {
  name: string;
  onClose: () => void;
  onSubmit: (password: string) => void;
}) {
  const [pw, setPw] = useState("");
  return (
    <Dialog
      title={`Password for ${name}`}
      onClose={onClose}
      width={420}
      footer={
        <>
          <span className="spacer" />
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" onClick={() => onSubmit(pw)}>
            Connect
          </button>
        </>
      }
    >
      <input
        className="input mono"
        type="password"
        value={pw}
        onChange={(e) => setPw(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            onSubmit(pw);
          }
        }}
        autoComplete="off"
      />
      <p className="muted small">This connection asks for its password every time; it isn't saved.</p>
    </Dialog>
  );
}
