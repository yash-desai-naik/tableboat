import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { errorText } from "../lib/connections";
import { documentApi, queryApi, type ResultInfo } from "../lib/query";
import { Dialog } from "../shell/Dialog";
import { Menu, type MenuItem } from "../shell/Menu";

/**
 * MongoDB results as documents (the default for MongoDB; the table is the other view).
 * Documents come from Rust as canonical Extended JSON, so every type is known here
 * (ObjectId, dates, NumberLong, decimals…) and big numbers are never rounded by JavaScript.
 * They load a page at a time as the list scrolls.
 */

/** Collection views can change documents; query results are read-only. */
export interface DocumentEditing {
  sessionId: string;
  database: string;
  collection: string;
  /** Reload the page after a change. */
  onChanged: () => void;
}

interface Props {
  info: ResultInfo;
  /** Documents in the current view (after quick search). */
  rowCount: number;
  /** Changes when quick search or sort changes the view. */
  version: number;
  onStatus: (text: string) => void;
  edit?: DocumentEditing;
}

const PAGE = 50;

type Json = null | boolean | number | string | Json[] | { [k: string]: Json };
type Obj = { [k: string]: Json };

export function DocumentList({ info, rowCount, version, onStatus, edit }: Props) {
  const [docs, setDocs] = useState<Json[]>([]);
  const [loading, setLoading] = useState(false);
  const [open, setOpen] = useState<Set<string>>(() => new Set());
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const [editor, setEditor] = useState<{ mode: "edit" | "insert"; index?: number; id?: Json; text: string } | null>(null);
  const sentinel = useRef<HTMLDivElement>(null);
  // MongoDB documents, or SQL rows shown as JSON objects.
  const noun = info.documents ? "document" : "row";
  // One loader for the first page and every later one: one request at a time, and a page is
  // only added if it starts where the list ends and the view hasn't changed meanwhile (the
  // first page and "scrolled to the end" used to both load page 1, showing documents twice).
  const loaded = useRef<{ view: string; count: number; busy: boolean }>({ view: "", count: 0, busy: false });
  const viewKey = `${info.id}:${version}`;
  const latest = useRef({ viewKey, rowCount, onStatus });
  latest.current = { viewKey, rowCount, onStatus };

  const load = useCallback(async () => {
    const state = loaded.current;
    const { viewKey: key, rowCount: total } = latest.current;
    if (state.busy || state.view !== key || state.count >= total) return;
    state.busy = true;
    setLoading(true);
    const start = state.count;
    try {
      const next = (await queryApi.documents(info.id, start, PAGE)) as Json[];
      if (loaded.current.view === key && loaded.current.count === start) {
        loaded.current.count = start + next.length;
        setDocs((d) => (start === 0 ? next : [...d, ...next]));
      }
    } catch (err) {
      latest.current.onStatus(errorText(err));
    } finally {
      state.busy = false;
      setLoading(false);
    }
  }, [info.id]);

  // A new view (search, sort, reload): start again from the top.
  useEffect(() => {
    loaded.current = { view: viewKey, count: 0, busy: false };
    setOpen(new Set());
    setDocs([]);
    void load();
  }, [viewKey, load]);

  // Load the next page when the end of the list scrolls into view.
  useEffect(() => {
    const el = sentinel.current;
    if (!el) return;
    const io = new IntersectionObserver((e) => e[0]?.isIntersecting && void load(), { rootMargin: "400px" });
    io.observe(el);
    return () => io.disconnect();
  }, [load, docs.length]);

  const toggle = (key: string) =>
    setOpen((s) => {
      const n = new Set(s);
      if (n.has(key)) n.delete(key);
      else n.add(key);
      return n;
    });

  async function copy(index: number, format: "shell" | "json") {
    try {
      await navigator.clipboard.writeText(await queryApi.documentText(info.id, index, format));
      onStatus(format === "json" ? "Copied as JSON" : "Copied in mongosh syntax");
    } catch (err) {
      onStatus(errorText(err));
    }
  }

  async function startEdit(index: number, mode: "edit" | "duplicate") {
    try {
      const text = await queryApi.documentText(info.id, index, mode === "edit" ? "shell" : "new");
      const id = (docs[index] as Obj | undefined)?._id;
      setEditor(mode === "edit" ? { mode: "edit", index, id, text } : { mode: "insert", text });
    } catch (err) {
      onStatus(errorText(err));
    }
  }

  async function remove(index: number) {
    if (!edit) return;
    const id = (docs[index] as Obj | undefined)?._id;
    if (id === undefined) return onStatus("This document has no _id.");
    const ok = await ask(`Delete the document with _id ${plain(id)} from ${edit.collection}?`, {
      title: "Delete document",
      kind: "warning",
      okLabel: "Delete",
    });
    if (!ok) return;
    try {
      await documentApi.remove(edit.sessionId, edit.database, edit.collection, id);
      onStatus("Deleted 1 document");
      edit.onChanged();
    } catch (err) {
      onStatus(errorText(err));
    }
  }

  function actions(index: number, x: number, y: number) {
    const items: MenuItem[] = [{ label: "Copy as JSON", onSelect: () => void copy(index, "json") }];
    if (info.documents) items.push({ label: "Copy in mongosh syntax", onSelect: () => void copy(index, "shell") });
    if (edit) {
      items.push(
        { label: "Edit…", onSelect: () => void startEdit(index, "edit") },
        { label: "Duplicate…", onSelect: () => void startEdit(index, "duplicate") },
        { label: "Delete…", onSelect: () => void remove(index) },
      );
    }
    setMenu({ x, y, items });
  }

  return (
    <div className="docs">
      {edit && (
        <div className="docs-head">
          <button className="btn small-btn" onClick={() => setEditor({ mode: "insert", text: "{\n  \n}" })}>
            Add document
          </button>
          <span className="muted small">Double-click a document to edit it.</span>
        </div>
      )}
      {docs.length === 0 && rowCount === 0 && <div className="results-note">No {noun}s.</div>}
      {docs.map((d, i) => (
        <div
          key={i}
          className="doc"
          onDoubleClick={(e) => {
            if (edit && !(e.target as HTMLElement).closest("button")) void startEdit(i, "edit");
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            actions(i, e.clientX, e.clientY);
          }}
        >
          <div className="doc-tools">
            <span className="doc-n">{i + 1}</span>
            <button className="icon-btn doc-more" title={edit ? "Copy, edit…" : "Copy"} onClick={(e) => actions(i, e.clientX - 180, e.clientY + 8)}>
              ⋯
            </button>
          </div>
          {isObj(d) ? <Fields obj={d} path={`${i}`} open={open} toggle={toggle} /> : <Value v={d} path={`${i}`} open={open} toggle={toggle} />}
        </div>
      ))}
      <div ref={sentinel} className="docs-end">
        {docs.length < rowCount ? (loading ? "Loading…" : "") : rowCount > 0 ? `${rowCount.toLocaleString()} ${noun}${rowCount === 1 ? "" : "s"}` : ""}
      </div>
      {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      {editor && edit && (
        <DocumentEditor
          title={editor.mode === "edit" ? `Edit document in ${edit.collection}` : `Add document to ${edit.collection}`}
          initial={editor.text}
          saveLabel={editor.mode === "edit" ? "Save" : "Add"}
          onClose={() => setEditor(null)}
          onSave={async (text) => {
            if (editor.mode === "edit") {
              await documentApi.replace(edit.sessionId, edit.database, edit.collection, editor.id, text);
              onStatus("Saved 1 document");
            } else {
              const id = await documentApi.insert(edit.sessionId, edit.database, edit.collection, text);
              onStatus(`Added a document with _id ${id}`);
            }
            setEditor(null);
            edit.onChanged();
          }}
        />
      )}
    </div>
  );
}

function DocumentEditor({
  title,
  initial,
  saveLabel,
  onClose,
  onSave,
}: {
  title: string;
  initial: string;
  saveLabel: string;
  onClose: () => void;
  onSave: (text: string) => Promise<void>;
}) {
  const [text, setText] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      await onSave(text);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setSaving(false);
    }
  };
  return (
    <Dialog
      title={title}
      onClose={onClose}
      width={760}
      footer={
        <>
          <span className="muted small">mongosh syntax: ObjectId("…"), ISODate("…"), NumberLong("…"), NumberDecimal("…")</span>
          <span className="spacer" />
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" disabled={saving} onClick={() => void save()}>
            {saving ? "Saving…" : saveLabel}
          </button>
        </>
      }
    >
      <textarea
        className="doc-editor"
        value={text}
        spellCheck={false}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "s" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            void save();
          } else if (e.key === "Tab" && !e.shiftKey) {
            e.preventDefault();
            const t = e.currentTarget;
            const at = t.selectionStart;
            setText(text.slice(0, at) + "  " + text.slice(t.selectionEnd));
            requestAnimationFrame(() => t.setSelectionRange(at + 2, at + 2));
          }
        }}
      />
      {error && <div className="edit-error doc-editor-error">{error}</div>}
    </Dialog>
  );
}

// ---------- rendering values ----------

function isObj(v: Json): v is Obj {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** An Extended JSON wrapper ({"$oid": …}, {"$date": …}…) shown as one value. */
function special(v: Json): { text: string; cls: string; title: string } | null {
  if (!isObj(v)) return null;
  const keys = Object.keys(v);
  if (keys.length === 0 || keys.length > 2 || !keys[0].startsWith("$")) return null;
  const x = v[keys[0]];
  switch (keys[0]) {
    case "$oid":
      return { text: `ObjectId("${x}")`, cls: "t-oid", title: "ObjectId" };
    case "$date": {
      const ms = isObj(x) && typeof x.$numberLong === "string" ? Number(x.$numberLong) : typeof x === "string" ? Date.parse(x) : NaN;
      const d = new Date(ms);
      return { text: Number.isNaN(d.getTime()) ? `ISODate(${JSON.stringify(x)})` : `ISODate("${d.toISOString()}")`, cls: "t-date", title: "Date" };
    }
    // SQL numbers, kept as text so JavaScript never rounds them.
    case "$number":
      return { text: String(x), cls: "t-num", title: "Number" };
    case "$numberInt":
      return { text: String(x), cls: "t-num", title: "Int32" };
    case "$numberLong":
      return { text: String(x), cls: "t-num", title: "Int64 (NumberLong)" };
    case "$numberDouble": {
      const s = String(x);
      return { text: /^-?\d+$/.test(s) ? `${s}.0` : s, cls: "t-num", title: "Double" };
    }
    case "$numberDecimal":
      return { text: String(x), cls: "t-num", title: "Decimal128" };
    case "$binary": {
      if (!isObj(x)) return null;
      const b64 = String(x.base64 ?? "");
      if (x.subType === "04") return { text: `UUID("${uuidFromBase64(b64)}")`, cls: "t-oid", title: "UUID" };
      return { text: `BinData(${parseInt(String(x.subType), 16)}, "${b64.length > 60 ? `${b64.slice(0, 60)}…` : b64}")`, cls: "t-oid", title: "Binary" };
    }
    case "$timestamp":
      return isObj(x) ? { text: `Timestamp(${x.t}, ${x.i})`, cls: "t-oid", title: "Timestamp" } : null;
    case "$regularExpression":
      return isObj(x) ? { text: `/${x.pattern}/${x.options}`, cls: "t-str", title: "Regular expression" } : null;
    case "$minKey":
      return { text: "MinKey()", cls: "t-kw", title: "MinKey" };
    case "$maxKey":
      return { text: "MaxKey()", cls: "t-kw", title: "MaxKey" };
    case "$undefined":
      return { text: "undefined", cls: "t-kw", title: "undefined" };
    default:
      return null;
  }
}

function uuidFromBase64(b64: string): string {
  try {
    const hex = Array.from(atob(b64), (c) => c.charCodeAt(0).toString(16).padStart(2, "0")).join("");
    return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
  } catch {
    return b64;
  }
}

/** A value as one short line of text (for questions and collapsed previews). */
function plain(v: Json): string {
  const s = special(v);
  if (s) return s.text;
  if (typeof v === "string") return JSON.stringify(v);
  if (Array.isArray(v)) return `[ ${v.length} item${v.length === 1 ? "" : "s"} ]`;
  if (isObj(v)) return `{ ${Object.keys(v).length} field${Object.keys(v).length === 1 ? "" : "s"} }`;
  return String(v);
}

/** Small objects and arrays of plain values stay on one line. */
function inline(v: Json): string | null {
  if (special(v)) return null;
  if (Array.isArray(v)) {
    if (!v.every((x) => (!isObj(x) && !Array.isArray(x)) || special(x))) return null;
    const s = `[${v.map(plain).join(", ")}]`;
    return s.length <= 80 ? s : null;
  }
  if (isObj(v)) {
    const entries = Object.entries(v);
    if (entries.length === 0) return "{}";
    if (entries.length > 4 || !entries.every(([, x]) => (!isObj(x) && !Array.isArray(x)) || special(x))) return null;
    const s = `{ ${entries.map(([k, x]) => `${k}: ${plain(x)}`).join(", ")} }`;
    return s.length <= 80 ? s : null;
  }
  return null;
}

interface TreeProps {
  path: string;
  open: Set<string>;
  toggle: (key: string) => void;
}

function Fields({ obj, ...tree }: { obj: Obj } & TreeProps) {
  return (
    <div className="doc-fields">
      {Object.entries(obj).map(([k, v]) => (
        <Field key={k} name={k} v={v} {...tree} path={`${tree.path}.${k}`} />
      ))}
    </div>
  );
}

function Field({ name, v, path, open, toggle }: { name: ReactNode; v: Json } & TreeProps) {
  const nested = (isObj(v) && !special(v)) || Array.isArray(v);
  const short = nested ? inline(v) : null;
  const expanded = open.has(path);
  if (nested && short === null) {
    return (
      <div className="doc-field">
        <button className="doc-caret" onClick={() => toggle(path)} aria-expanded={expanded}>
          <span className={expanded ? "caret open" : "caret"}>▸</span>
          <span className="doc-key">{name}</span>
          <span className="doc-colon">: </span>
          {!expanded && <span className="doc-sum">{plain(v)}</span>}
        </button>
        {expanded &&
          (Array.isArray(v) ? (
            <div className="doc-fields">
              {v.map((x, i) => (
                <Field key={i} name={<span className="doc-index">{i}</span>} v={x} path={`${path}.${i}`} open={open} toggle={toggle} />
              ))}
            </div>
          ) : (
            <Fields obj={v as Obj} path={path} open={open} toggle={toggle} />
          ))}
      </div>
    );
  }
  return (
    <div className="doc-field">
      <span className="doc-key">{name}</span>
      <span className="doc-colon">: </span>
      {short !== null ? <span className="doc-inline">{short}</span> : <Value v={v} path={path} open={open} toggle={toggle} />}
    </div>
  );
}

const LONG = 300;

function Value({ v, path, open, toggle }: { v: Json } & TreeProps) {
  const s = special(v);
  if (s)
    return (
      <span className={s.cls} title={s.title}>
        {s.text}
      </span>
    );
  if (v === null) return <span className="t-kw">null</span>;
  if (typeof v === "boolean") return <span className="t-kw">{String(v)}</span>;
  if (typeof v === "number") return <span className="t-num">{String(v)}</span>;
  if (typeof v === "string") {
    const full = open.has(path);
    if (v.length > LONG && !full)
      return (
        <span className="t-str">
          {JSON.stringify(v.slice(0, LONG)).slice(0, -1)}…
          <button className="doc-more-text" onClick={() => toggle(path)}>
            {(v.length - LONG).toLocaleString()} more characters
          </button>
        </span>
      );
    return <span className="t-str">{JSON.stringify(v)}</span>;
  }
  return <span className="doc-inline">{plain(v)}</span>;
}
