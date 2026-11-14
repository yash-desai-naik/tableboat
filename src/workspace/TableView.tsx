import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import type { SessionInfo } from "../lib/connections";
import { errorText } from "../lib/connections";
import {
  addRow,
  deleteRows,
  discardPending,
  duplicateRows,
  editApi,
  hasPending,
  pendingCount,
  tableActions,
  toChanges,
  useEditVersion,
  type EditContext,
  type TableColumn,
} from "../lib/edits";
import { quoteIdent } from "../lib/meta";
import { fmtShortcut } from "../lib/platform";
import { dialectOf, queryApi, runForTab, useRunState } from "../lib/query";
import { useSettings } from "../lib/settings";
import type { ObjectTab } from "../lib/workspace";
import { openQuery } from "../editor/actions";
import { gridHandles, type SortState } from "../results/ResultGrid";
import { ResultsPane } from "../results/ResultsPane";
import { onRefresh } from "../lib/refresh";
import { Dialog } from "../shell/Dialog";
import {
  ChevronLeftIcon,
  ChevronRightIcon,
  CodeIcon,
  DuplicateIcon,
  PlusIcon,
  RefreshIcon,
  TrashIcon,
} from "../shell/Icons";
import { ObjectView } from "./ObjectView";

/** Data / Definition choice and paging per tab, kept while switching tabs. */
const modes = new Map<string, "data" | "definition">();
const paging = new Map<string, { offset: number; sort: SortState }>();
const columnCache = new Map<string, TableColumn[]>();
// Column details (keys, types) may have changed on the server: forget them on Refresh.
onRefresh(() => columnCache.clear());

/** A table or view: its rows a page at a time (sorted on the server), or its definition. */
export function TableView({ info, tab }: { info: SessionInfo; tab: ObjectTab }) {
  const hasData = tab.kind === "table" || tab.kind === "view";
  const [mode, setMode] = useState<"data" | "definition">(modes.get(tab.id) ?? (hasData ? "data" : "definition"));

  const choose = (m: "data" | "definition") => {
    modes.set(tab.id, m);
    setMode(m);
  };

  const switcher = hasData ? (
    <div className="segmented small-seg" role="tablist">
      <button role="tab" aria-selected={mode === "data"} className={mode === "data" ? "seg active" : "seg"} onClick={() => choose("data")}>
        Data
      </button>
      <button
        role="tab"
        aria-selected={mode === "definition"}
        className={mode === "definition" ? "seg active" : "seg"}
        onClick={() => choose("definition")}
      >
        Definition
      </button>
    </div>
  ) : null;

  return (
    <div className="table-view">
      {mode === "data" && hasData ? (
        <TableData info={info} tab={tab} switcher={switcher} />
      ) : (
        <>
          {switcher && <div className="view-switch">{switcher}</div>}
          <ObjectView sessionId={info.sessionId} tab={tab} />
        </>
      )}
    </div>
  );
}

function TableData({ info, tab, switcher }: { info: SessionInfo; tab: ObjectTab; switcher: ReactNode }) {
  const settings = useSettings();
  const run = useRunState(tab.id);
  const editVersion = useEditVersion(tab.id);
  const engine = info.server.engine ?? info.connection.engine;
  const dialect = dialectOf(engine);
  const pageSize = settings.pageSize;
  const saved = paging.get(tab.id);
  const [offset, setOffset] = useState(saved?.offset ?? 0);
  const [sort, setSort] = useState<SortState>(saved?.sort ?? null);
  const [total, setTotal] = useState<number | null>(null);
  const [counting, setCounting] = useState(false);
  const [tableCols, setTableCols] = useState<TableColumn[] | null>(columnCache.get(tab.id) ?? null);
  const [saving, setSaving] = useState(false);
  const [editError, setEditError] = useState<string | null>(null);
  const [preview, setPreview] = useState<string[] | null>(null);

  const q = (s: string) => quoteIdent(s, engine);
  const isMongo = engine === "mongodb";
  const from = `${q(tab.database)}.${q(tab.name)}`;
  const coll = `db.getCollection(${JSON.stringify(tab.name)})`;
  const resultInfo = run.outcomes[0]?.resultInfo ?? null;
  const resultNames = useMemo(() => resultInfo?.columns.map((c) => c.name) ?? [], [resultInfo]);

  // Column details (primary key, auto-increment, generated) decide what can be edited.
  useEffect(() => {
    if (tab.kind !== "table" || columnCache.has(tab.id)) return;
    editApi
      .columns(info.sessionId, tab.database, tab.name)
      .then((cols) => {
        columnCache.set(tab.id, cols);
        setTableCols(cols);
      })
      .catch(() => setTableCols([]));
  }, [tab.id, tab.kind, tab.database, tab.name, info.sessionId]);

  const edit = useMemo((): { ctx: EditContext } | { reason: string } => {
    if (tab.kind !== "table") return { reason: "Views are read-only." };
    if (!tableCols || !resultInfo) return { reason: "" };
    const pk = tableCols.filter((c) => c.primaryKey);
    if (!pk.length) return { reason: "Read-only: this table has no primary key, so rows can't be changed safely." };
    const keyIdx = pk.map((c) => resultNames.indexOf(c.name));
    if (keyIdx.some((i) => i < 0)) return { reason: "Read-only: the primary key isn't in the result." };
    return { ctx: { keyIdx, names: resultNames, columns: resultNames.map((n) => tableCols.find((c) => c.name === n)) } };
  }, [tab.kind, tableCols, resultInfo, resultNames]);
  const ctx = "ctx" in edit ? edit.ctx : null;

  const buildSql = useCallback(
    (off: number, s: SortState) => {
      if (isMongo) {
        const sort = s && resultNames[s.column] ? `.sort({ ${JSON.stringify(resultNames[s.column])}: ${s.descending ? -1 : 1} })` : "";
        return `${coll}.find({})${sort}.skip(${off}).limit(${pageSize})`;
      }
      const order = s && resultNames[s.column] ? ` ORDER BY ${q(resultNames[s.column])} ${s.descending ? "DESC" : "ASC"}` : "";
      return `SELECT * FROM ${from}${order} LIMIT ${pageSize} OFFSET ${off}`;
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [from, pageSize, engine, resultNames],
  );

  const load = useCallback(
    async (off: number, s: SortState) => {
      paging.set(tab.id, { offset: off, sort: s });
      await runForTab(tab.id, {
        sessionId: info.sessionId,
        statements: [buildSql(off, s)],
        database: isMongo ? tab.database : null,
        maxRows: pageSize,
        usePool: true,
        stopOnError: true,
      });
    },
    [tab.id, info.sessionId, buildSql, pageSize],
  );

  // Refresh (Ctrl+R): column details and the current page again (asks first about unsaved edits).
  useEffect(
    () =>
      onRefresh(() => {
        if (tab.kind === "table") {
          editApi
            .columns(info.sessionId, tab.database, tab.name)
            .then((cols) => {
              columnCache.set(tab.id, cols);
              setTableCols(cols);
            })
            .catch(() => setTableCols([]));
        }
        setTotal(null);
        void go(offset);
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tab.id, offset, sort, info.sessionId],
  );

  // Load when first shown; a tab that already has rows (from before a tab switch) keeps them.
  useEffect(() => {
    if (run.outcomes.length || run.running) return;
    void load(offset, sort);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.id]);

  /** Page, sort or reload: unsaved edits would be lost, so ask first. */
  async function confirmLeave(): Promise<boolean> {
    if (!hasPending(tab.id)) return true;
    const n = pendingCount(tab.id);
    const ok = await ask(`Discard ${n} unsaved change${n === 1 ? "" : "s"} in ${tab.name}?`, {
      title: "Unsaved changes",
      kind: "warning",
      okLabel: "Discard",
      cancelLabel: "Keep editing",
    });
    if (ok) discardPending(tab.id);
    return ok;
  }

  async function go(off: number, s: SortState = sort) {
    if (!(await confirmLeave())) return;
    setOffset(off);
    setSort(s);
    void load(off, s);
  }

  async function save() {
    if (!ctx || !hasPending(tab.id) || saving) return;
    setSaving(true);
    setEditError(null);
    try {
      await editApi.save(info.sessionId, tab.database, tab.name, toChanges(tab.id, ctx));
      discardPending(tab.id);
      setPreview(null);
      setTotal(null);
      await load(offset, sort);
    } catch (err) {
      setEditError(errorText(err));
    } finally {
      setSaving(false);
    }
  }

  async function showPreview() {
    if (!ctx || !hasPending(tab.id)) return;
    try {
      setPreview(await editApi.preview(info.sessionId, tab.database, tab.name, toChanges(tab.id, ctx)));
    } catch (err) {
      setEditError(errorText(err));
    }
  }

  async function duplicate() {
    if (!ctx) return;
    const t = await gridHandles.get(tab.id)?.targets();
    if (!t || (!t.existing.length && !t.inserts.length)) {
      setEditError("Select one or more rows to duplicate (click a row number, or select cells in those rows).");
      return;
    }
    setEditError(null);
    duplicateRows(tab.id, ctx, t.existing, t.inserts);
  }

  async function remove() {
    if (!ctx) return;
    const t = await gridHandles.get(tab.id)?.targets();
    if (!t || (!t.existing.length && !t.inserts.length)) {
      setEditError("Select the rows to delete first.");
      return;
    }
    setEditError(null);
    deleteRows(tab.id, ctx, t.existing, t.inserts);
  }

  // Shortcuts (Ctrl+S, Ctrl+I, Ctrl+Shift+Delete) reach the open table through this.
  useEffect(() => {
    tableActions.set(tab.id, {
      reload: () => void go(offset),
      save: () => void save(),
      discard: () => discardPending(tab.id),
      preview: () => void showPreview(),
      addRow: () => ctx && addRow(tab.id, ctx),
      duplicate: () => void duplicate(),
      deleteRows: () => void remove(),
    });
    return () => {
      tableActions.delete(tab.id);
    };
  });

  async function countRows() {
    setCounting(true);
    try {
      const out = await queryApi.run({
        sessionId: info.sessionId,
        runId: crypto.randomUUID(),
        statements: [isMongo ? `${coll}.countDocuments({})` : `SELECT COUNT(*) FROM ${from}`],
        database: isMongo ? tab.database : null,
        maxRows: 1,
        usePool: true,
        stopOnError: true,
      });
      const r = out[0]?.resultInfo;
      if (r) {
        const rows = await queryApi.rows(r.id, 0, 1);
        setTotal(Number(rows[0]?.[0] ?? 0));
        void queryApi.free([r.id]);
      } else if (out[0]?.error) {
        setEditError(out[0].error);
      }
    } catch (err) {
      setEditError(errorText(err));
    } finally {
      setCounting(false);
    }
  }

  const count = pendingCount(tab.id);
  const loaded = resultInfo?.rowCount ?? 0;
  const hasNext = loaded >= pageSize;
  const rangeText = loaded ? `${(offset + 1).toLocaleString()}–${(offset + loaded).toLocaleString()}` : "0";

  // Status bar, right: ‹ 1–300 of 12,345 ›  Count
  const pager = (
    <div className="pager">
      <button className="icon-btn small" disabled={offset === 0 || run.running} onClick={() => void go(Math.max(0, offset - pageSize))} title="Previous page">
        <ChevronLeftIcon size={14} />
      </button>
      <span className="pager-text">
        {rangeText}
        {total !== null ? ` of ${total.toLocaleString()}` : ""}
      </span>
      <button className="icon-btn small" disabled={!hasNext || run.running} onClick={() => void go(offset + pageSize)} title="Next page">
        <ChevronRightIcon size={14} />
      </button>
      <button className="link-btn pager-count" disabled={counting} onClick={() => void countRows()} title="Count all rows (SELECT COUNT(*))">
        {counting ? "Counting…" : "Count"}
      </button>
    </div>
  );

  // Toolbar, left: Data/Definition, then row actions as icons; Save… appears with unsaved edits.
  const leading = (
    <>
      {switcher}
      {ctx ? (
        <div className="tool-group">
          <button className="icon-btn" onClick={() => addRow(tab.id, ctx)} title={`Add a row (${fmtShortcut("Ctrl+I")})`}>
            <PlusIcon />
          </button>
          <button className="icon-btn" onClick={() => void duplicate()} title={`Duplicate the selected rows (${fmtShortcut("Ctrl+D")})`}>
            <DuplicateIcon />
          </button>
          <button className="icon-btn danger" onClick={() => void remove()} title="Delete the selected rows (Delete); press again to undo">
            <TrashIcon />
          </button>
        </div>
      ) : (
        "reason" in edit && edit.reason && <span className="muted small">{edit.reason}</span>
      )}
      {editError && (
        <span className="edit-error" title={editError}>
          {editError}
        </span>
      )}
      {ctx && count > 0 && (
        <div className="tool-group pending" data-version={editVersion}>
          <button className="btn small-btn" onClick={() => void showPreview()} title="Show the SQL that Save will run">
            Preview
          </button>
          <button
            className="btn small-btn"
            onClick={() => discardPending(tab.id)}
            title={`Throw away all unsaved changes (${fmtShortcut("Ctrl+Shift+Delete")})`}
          >
            Discard
          </button>
          <button
            className="btn primary small-btn"
            disabled={saving}
            onClick={() => void save()}
            title={`Save every change in one transaction (${fmtShortcut("Ctrl+S")})`}
          >
            {saving ? "Saving…" : `Save ${count}`}
          </button>
        </div>
      )}
    </>
  );

  // Toolbar, right (before Export): reload and open as SQL.
  const tools = (
    <>
      <button
        className="icon-btn"
        disabled={run.running}
        onClick={() => void go(offset)}
        title={`Reload this page (${fmtShortcut("Ctrl+R")})`}
      >
        <RefreshIcon />
      </button>
      <button className="icon-btn" onClick={() => openQuery(`${buildSql(offset, sort)};`)} title="Open this query in a SQL tab">
        <CodeIcon />
      </button>
    </>
  );

  return (
    <>
      <div className="table-data">
        <ResultsPane
          tabId={tab.id}
          dialect={dialect}
          table={tab.name}
          serverSort={{ sort, onSort: (s) => void go(0, s) }}
          leading={leading}
          tools={tools}
          footer={pager}
          emptyText="Loading…"
          edit={ctx ? { tabId: tab.id, ctx, version: editVersion, dialect, engine } : undefined}
          documentEdit={
            isMongo && tab.kind === "table"
              ? { sessionId: info.sessionId, database: tab.database, collection: tab.name, onChanged: () => void go(offset) }
              : undefined
          }
        />
      </div>
      {preview && (
        <Dialog
          title={`Changes to ${tab.name}`}
          onClose={() => setPreview(null)}
          width={760}
          footer={
            <>
              <span className="muted small">Runs in one transaction; values are sent as parameters.</span>
              <span className="spacer" />
              <button className="btn" onClick={() => void navigator.clipboard.writeText(preview.join("\n"))}>
                Copy
              </button>
              <button className="btn" onClick={() => setPreview(null)}>
                Close
              </button>
              <button className="btn primary" disabled={saving} onClick={() => void save()}>
                {saving ? "Saving…" : `Save ${count}`}
              </button>
            </>
          }
        >
          <pre className="ddl preview-sql">{preview.join("\n")}</pre>
        </Dialog>
      )}
    </>
  );
}
