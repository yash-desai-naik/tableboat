import { useEffect, useRef, useState } from "react";
import { Command as Cmdk } from "cmdk";
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
import { NewWindowIcon } from "../shell/Icons";
import { ConnectionForm } from "./ConnectionForm";
import { PasswordDialog, UrlDialog } from "./ConnectionList";

/** "list": open here (Ctrl+Enter = new window); "window": open in a new window (Ctrl+Enter = here). */
export type SwitcherMode = "list" | "window" | "new" | "url";

interface Props {
  initialMode: SwitcherMode;
  onClose: () => void;
}

type Stage =
  | { kind: "list" }
  | { kind: "url" }
  | { kind: "form"; initial: Connection; secrets?: Secrets }
  | { kind: "password"; target: ConnectionSummary; newWindow: boolean };

/**
 * Ctrl+Shift+K: open another saved connection (Enter = this window, Ctrl+Enter = new
 * window), or create one, without going back to the start page.
 */
export function ConnectionSwitcher({ initialMode, onClose }: Props) {
  const session = useSession();
  const currentId = session.status === "open" ? session.info.connection.id : null;
  const [items, setItems] = useState<ConnectionSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [stage, setStage] = useState<Stage>(
    initialMode === "new" ? { kind: "form", initial: newConnection() } : initialMode === "url" ? { kind: "url" } : { kind: "list" },
  );
  const newWindow = useRef(false);
  const [busy, setBusy] = useState<string | null>(null);
  // In "window" mode Enter opens a new window and Ctrl+Enter opens here; otherwise the reverse.
  const windowFirst = initialMode === "window";

  useEffect(() => {
    api
      .list()
      .then((list) => setItems(list.sort((a, b) => displayName(a).localeCompare(displayName(b)))))
      .catch((err) => setError(errorText(err)));
  }, []);

  async function open(c: ConnectionSummary, inNewWindow: boolean, password?: string) {
    if (inNewWindow) {
      await api.newWindow(c.id);
      onClose();
      return;
    }
    // Stay open until connected, so a password prompt or an error can be shown here.
    setBusy(displayName(c));
    setError(null);
    try {
      await connect(c.id, displayName(c), password);
      onClose();
    } catch (err) {
      const text = errorText(err);
      setBusy(null);
      if (text === "PASSWORD_REQUIRED") setStage({ kind: "password", target: c, newWindow: false });
      else {
        setStage({ kind: "list" });
        setError(`${displayName(c)}: ${text}`);
      }
    }
  }

  if (stage.kind === "url") {
    return (
      <UrlDialog
        onClose={onClose}
        onParsed={(connection, secrets) => setStage({ kind: "form", initial: connection, secrets })}
      />
    );
  }

  if (stage.kind === "form") {
    return (
      <ConnectionForm
        title="New connection"
        initial={stage.initial}
        initialSecrets={stage.secrets}
        onClose={onClose}
        onSaved={(saved, connectWith) => {
          if (!connectWith) {
            onClose();
            return;
          }
          // Connect here; the previous connection in this window is closed.
          void open(saved, false, connectWith.password);
        }}
      />
    );
  }

  if (stage.kind === "password") {
    return (
      <PasswordDialog
        name={displayName(stage.target)}
        onClose={onClose}
        onSubmit={(pw) => void open(stage.target, stage.newWindow, pw)}
      />
    );
  }

  return (
    <div className="overlay" onMouseDown={onClose}>
      <div className="palette" onMouseDown={(e) => e.stopPropagation()}>
        <Cmdk label="Open connection" loop>
          <Cmdk.Input
            autoFocus
            placeholder={windowFirst ? "Open a connection in a new window…" : "Open a connection…"}
            onKeyDown={(e) => {
              newWindow.current = e.ctrlKey;
              if (e.key === "Escape") {
                e.preventDefault();
                onClose();
              }
            }}
          />
          <Cmdk.List>
            {busy && <div className="palette-note">Connecting to {busy}…</div>}
            {error && <div className="palette-note error">{error}</div>}
            {items === null && !error && <div className="palette-note">Loading…</div>}
            <Cmdk.Empty>No saved connection matches.</Cmdk.Empty>
            {items && items.length > 0 && (
              <Cmdk.Group heading="Saved connections">
                {items.map((c) => {
                  const tag = tagInfo(c.tag);
                  return (
                    <Cmdk.Item
                      key={c.id}
                      value={`${displayName(c)} ${describeTarget(c)} ${c.tag ?? ""} ${c.id}`}
                      onSelect={() => {
                        const ctrl = newWindow.current;
                        newWindow.current = false;
                        void open(c, windowFirst ? !ctrl : ctrl);
                      }}
                    >
                      <span className="conn-badge small" style={{ background: c.color }}>
                        {engineInfo(c.engine).badge}
                      </span>
                      <span className="palette-title">
                        {displayName(c)}
                        {c.id === currentId && <span className="palette-meta"> · open here</span>}
                      </span>
                      <span className="palette-meta">{describeTarget(c)}</span>
                      {tag && (
                        <span className="tag-chip" style={{ ["--tag" as string]: tag.color }}>
                          {tag.short}
                        </span>
                      )}
                      {!windowFirst && (
                        <button
                          className="icon-btn small palette-newwin"
                          title="Open in a new window (Ctrl+Enter)"
                          onMouseDown={(e) => e.stopPropagation()}
                          onPointerDown={(e) => e.stopPropagation()}
                          onClick={(e) => {
                            e.stopPropagation();
                            void open(c, true);
                          }}
                        >
                          <NewWindowIcon size={14} />
                        </button>
                      )}
                    </Cmdk.Item>
                  );
                })}
              </Cmdk.Group>
            )}
            <Cmdk.Group heading="Create">
              <Cmdk.Item value="__new New connection" onSelect={() => setStage({ kind: "form", initial: newConnection() })}>
                <span className="palette-check">+</span>
                <span className="palette-title">New connection…</span>
              </Cmdk.Item>
              <Cmdk.Item value="__url New connection from URL" onSelect={() => setStage({ kind: "url" })}>
                <span className="palette-check">+</span>
                <span className="palette-title">New connection from URL…</span>
              </Cmdk.Item>
            </Cmdk.Group>
          </Cmdk.List>
          <div className="palette-footer">
            <span>
              <kbd>↵</kbd> {windowFirst ? "New window" : "Open here"}
            </span>
            <span>
              <kbd>Ctrl</kbd>
              <kbd>↵</kbd> {windowFirst ? "Open here" : "New window"}
            </span>
            <span>
              <kbd>Esc</kbd> Close
            </span>
          </div>
        </Cmdk>
      </div>
    </div>
  );
}
