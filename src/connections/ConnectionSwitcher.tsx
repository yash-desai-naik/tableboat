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
import { ask } from "@tauri-apps/plugin-dialog";
import { applyEditedConnection, connect, useSession } from "../lib/session";
import { fmtShortcut, IS_MAC, modPressed } from "../lib/platform";
import { NewWindowIcon, PencilIcon } from "../shell/Icons";
import { ConnectionForm } from "./ConnectionForm";
import { PasswordDialog, UrlDialog } from "./ConnectionList";

/**
 * "list": open here (Ctrl+Enter = new window); "window": open in a new window (Ctrl+Enter = here);
 * "edit": edit the connection open in this window.
 */
export type SwitcherMode = "list" | "window" | "new" | "url" | "edit";

interface Props {
  initialMode: SwitcherMode;
  onClose: () => void;
}

type Stage =
  | { kind: "list" }
  | { kind: "url" }
  | { kind: "form"; initial: Connection; secrets?: Secrets }
  | { kind: "edit"; summary: ConnectionSummary }
  | { kind: "loading" }
  | { kind: "password"; target: ConnectionSummary; newWindow: boolean };

/** A connection row's search text (also its identity for the highlighted row). */
function itemValue(c: ConnectionSummary): string {
  return `${displayName(c)} ${describeTarget(c)} ${c.tag ?? ""} ${c.id}`.trim();
}

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
    initialMode === "new"
      ? { kind: "form", initial: newConnection() }
      : initialMode === "url"
        ? { kind: "url" }
        : initialMode === "edit"
          ? { kind: "loading" }
          : { kind: "list" },
  );
  const newWindow = useRef(false);
  const [busy, setBusy] = useState<string | null>(null);
  // In "window" mode Enter opens a new window and Ctrl+Enter opens here; otherwise the reverse.
  const windowFirst = initialMode === "window";
  // The highlighted row. The saved connections arrive after the "Create" items are on screen,
  // so pick a connection once they load: the first one not already open here.
  const [active, setActive] = useState("");
  useEffect(() => {
    if (!items?.length) return;
    const first = items.find((c) => c.id !== currentId) ?? items[0];
    setActive(itemValue(first));
  }, [items, currentId]);

  useEffect(() => {
    api
      .list()
      .then((list) => {
        setItems(list.sort((a, b) => displayName(a).localeCompare(displayName(b))));
        // "Edit connection": straight to the form for the connection open here.
        if (initialMode === "edit") {
          const current = list.find((c) => c.id === currentId);
          setStage(current ? { kind: "edit", summary: current } : { kind: "list" });
        }
      })
      .catch((err) => {
        setError(errorText(err));
        setStage((s) => (s.kind === "loading" ? { kind: "list" } : s));
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** After saving an edit: update this window if it's the open connection; offer to reconnect. */
  async function afterEdit(saved: ConnectionSummary, connectWith: { password?: string } | null) {
    if (connectWith) {
      void open(saved, false, connectWith.password);
      return;
    }
    setItems((list) => list && list.map((c) => (c.id === saved.id ? saved : c)));
    if (applyEditedConnection(saved)) {
      const reconnect = await ask(`${displayName(saved)}: the new settings take effect after reconnecting. Reconnect now?`, {
        title: "Connection changed",
        kind: "info",
        okLabel: "Reconnect",
        cancelLabel: "Later",
      });
      if (reconnect) {
        void open(saved, false);
        return;
      }
    }
    onClose();
  }

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

  if (stage.kind === "loading") return null;

  if (stage.kind === "edit") {
    return (
      <ConnectionForm
        title={`Edit ${displayName(stage.summary)}`}
        initial={stage.summary}
        stored={stage.summary}
        onClose={onClose}
        onSaved={(saved, connectWith) => void afterEdit(saved, connectWith)}
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
        <Cmdk label="Open connection" loop value={active} onValueChange={setActive}>
          <Cmdk.Input
            autoFocus
            placeholder={windowFirst ? "Open a connection in a new window…" : "Open a connection…"}
            onKeyDown={(e) => {
              newWindow.current = modPressed(e);
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
                      value={itemValue(c)}
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
                      <button
                        className="icon-btn small palette-action"
                        title="Edit this connection"
                        onMouseDown={(e) => e.stopPropagation()}
                        onPointerDown={(e) => e.stopPropagation()}
                        onClick={(e) => {
                          e.stopPropagation();
                          setStage({ kind: "edit", summary: c });
                        }}
                      >
                        <PencilIcon size={14} />
                      </button>
                      {!windowFirst && (
                        <button
                          className="icon-btn small palette-action"
                          title={`Open in a new window (${fmtShortcut("Ctrl+Enter")})`}
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
              <kbd>{IS_MAC ? "⌘" : "Ctrl"}</kbd>
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
