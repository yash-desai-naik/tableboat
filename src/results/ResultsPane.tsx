import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { save as saveFile } from "@tauri-apps/plugin-dialog";
import { errorText } from "../lib/connections";
import {
  formatElapsed,
  queryApi,
  rerunTab,
  setRunState,
  useRunState,
  type CopyFormat,
  type Dialect,
  type StatementOutcome,
} from "../lib/query";
import { DownloadIcon, RefreshIcon, SearchIcon } from "../shell/Icons";
import { Menu, type MenuItem } from "../shell/Menu";
import { ResultGrid, type GridEditing, type SortState } from "./ResultGrid";
import { DocumentList, type DocumentEditing } from "./DocumentList";
import { updateSettings, useSettings } from "../lib/settings";

interface Props {
  tabId: string;
  dialect: Dialect;
  /** Table name for "Copy as INSERT" when the results come from one table. */
  table: string | null;
  /** Server-side sorting (table views). Without it, sorting happens on the loaded rows. */
  serverSort?: { sort: SortState; onSort: (s: SortState) => void };
  onCancel?: () => void;
  /**
   * Table views: controls for the left of the single toolbar (Data/Definition, row actions,
   * Save…). With it, the row count and `footer` move to a status bar under the grid.
   */
  leading?: ReactNode;
  /** Icon buttons placed before Export (table views: reload, open as SQL). */
  tools?: ReactNode;
  /** Right side of the status bar (table views: the pager). */
  footer?: ReactNode;
  emptyText?: string;
  edit?: GridEditing;
  /** MongoDB collection views: edit, add and delete whole documents. */
  documentEdit?: DocumentEditing;
}

const EXPORTS: { format: CopyFormat; label: string; ext: string }[] = [
  { format: "csv-header", label: "CSV", ext: "csv" },
  { format: "xlsx", label: "Excel", ext: "xlsx" },
  { format: "json", label: "JSON", ext: "json" },
  { format: "markdown", label: "Markdown", ext: "md" },
  { format: "insert", label: "SQL INSERT", ext: "sql" },
  { format: "tsv-header", label: "Tab-separated", ext: "tsv" },
];

/** Per-result view state: quick search, sort, visible row count. */
interface ViewState {
  search: string;
  sort: SortState;
  rowCount: number;
  version: number;
}

export function ResultsPane({ tabId, dialect, table, serverSort, onCancel, leading, tools, footer, emptyText, edit, documentEdit }: Props) {
  const run = useRunState(tabId);
  const { mongoView } = useSettings();
  const [views, setViews] = useState<Record<string, ViewState>>({});
  const [status, setStatus] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const statusTimer = useRef<number | undefined>(undefined);

  const outcome: StatementOutcome | undefined = run.active >= 0 ? run.outcomes[run.active] : undefined;
  const info = outcome?.resultInfo ?? null;
  const view: ViewState | null = info
    ? (views[info.id] ?? { search: "", sort: null, rowCount: info.rowCount, version: 0 })
    : null;
  // MongoDB results show as documents unless the table was chosen.
  const asDocuments = !!info?.documents && mongoView === "documents";

  // Elapsed time while running; the timer only exists during a query.
  useEffect(() => {
    if (!run.running) return;
    const t = window.setInterval(() => setElapsed(performance.now() - run.startedAt), 250);
    return () => window.clearInterval(t);
  }, [run.running, run.startedAt]);

  function flash(text: string) {
    setStatus(text);
    window.clearTimeout(statusTimer.current);
    statusTimer.current = window.setTimeout(() => setStatus(null), 2500);
  }

  function updateView(id: string, patch: Partial<ViewState>) {
    setViews((v) => {
      const base = v[id] ?? { search: "", sort: null, rowCount: info?.rowCount ?? 0, version: 0 };
      return { ...v, [id]: { ...base, ...patch, version: base.version + 1 } };
    });
  }

  async function applySearch(text: string) {
    if (!info) return;
    try {
      const count = await queryApi.filter(info.id, text || null);
      updateView(info.id, { search: text, rowCount: count });
    } catch (err) {
      flash(errorText(err));
    }
  }

  async function applySort(sort: SortState) {
    if (!info) return;
    if (serverSort) {
      serverSort.onSort(sort);
      return;
    }
    try {
      const count = await queryApi.sort(info.id, sort?.column ?? null, sort?.descending ?? false);
      updateView(info.id, { sort, rowCount: count });
    } catch (err) {
      flash(errorText(err));
    }
  }

  async function exportAs(format: CopyFormat, ext: string) {
    if (!info) return;
    const path = await saveFile({
      title: "Export results",
      defaultPath: `${table ?? "results"}.${ext}`,
      filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
    });
    if (!path) return;
    try {
      const n = await queryApi.exportFile({ resultId: info.id, path, format, dialect, table });
      flash(`Exported ${n.toLocaleString()} rows to ${path}`);
    } catch (err) {
      flash(errorText(err));
    }
  }

  function exportMenu(x: number, y: number) {
    setMenu({ x, y, items: EXPORTS.map((e) => ({ label: `Export as ${e.label}…`, onSelect: () => void exportAs(e.format, e.ext) })) });
  }

  const tabs = useMemo(
    () => run.outcomes.map((o, i) => ({ i, o })).filter(({ o }) => o.resultInfo),
    [run.outcomes],
  );
  const errors = run.outcomes.filter((o) => o.error && !o.cancelled).length;

  const unit = info?.documents ? "document" : "row";
  const summary = (() => {
    if (run.running) return `Running… ${formatElapsed(elapsed)}`;
    if (!outcome) {
      if (!run.outcomes.length) return "";
      const total = run.outcomes.reduce((a, o) => a + o.elapsedMs, 0);
      return `${run.outcomes.length} statement${run.outcomes.length === 1 ? "" : "s"} · ${formatElapsed(total)}`;
    }
    if (!info) return "";
    const shown = view?.rowCount ?? info.rowCount;
    const rows =
      shown !== info.rowCount
        ? `${shown.toLocaleString()} of ${info.rowCount.toLocaleString()} ${unit}s`
        : `${info.rowCount.toLocaleString()} ${unit}${info.rowCount === 1 ? "" : "s"}`;
    return `${rows} · ${formatElapsed(outcome.elapsedMs)}`;
  })();

  const searchBox = info && (
    <label className="search-box" title="Search the loaded rows (Esc clears)">
      <SearchIcon size={13} />
      <input
        className="search-input"
        placeholder="Search"
        value={view?.search ?? ""}
        onChange={(e) => void applySearch(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape" && view?.search) {
            e.stopPropagation();
            void applySearch("");
          }
        }}
        spellCheck={false}
      />
    </label>
  );

  const viewSwitch = info?.documents && (
    <div className="segmented small-seg" role="tablist" aria-label="Show results as">
      {(["documents", "table"] as const).map((v) => (
        <button
          key={v}
          role="tab"
          aria-selected={mongoView === v}
          className={mongoView === v ? "seg active" : "seg"}
          onClick={() => updateSettings({ mongoView: v })}
          title={v === "documents" ? "Documents as they are stored (nesting and types)" : "Fields as columns"}
        >
          {v === "documents" ? "Documents" : "Table"}
        </button>
      ))}
    </div>
  );

  const exportButton = info && (
    <button
      className="icon-btn"
      title="Export…"
      onClick={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        exportMenu(r.right - 200, r.bottom + 4);
      }}
    >
      <DownloadIcon />
    </button>
  );

  const summaryText = (
    <>
      {summary}
      {info?.truncated && (
        <span className="trunc" title="Only the first rows were kept. Raise the limit in settings or add LIMIT/WHERE.">
          {" "}
          · first {info.rowCount.toLocaleString()} only
        </span>
      )}
    </>
  );

  const body = (
    <div className="results-body">
      {run.running && !run.outcomes.length && <div className="results-note">Running…</div>}
      {!run.running && !run.outcomes.length && <div className="results-note">{emptyText ?? "Run a query to see results here."}</div>}
      {info && view && asDocuments && (
        <DocumentList
          key={info.id}
          info={info}
          rowCount={view.rowCount}
          version={view.version}
          onStatus={flash}
          edit={documentEdit}
        />
      )}
      {info && view && !asDocuments && (
        <ResultGrid
          key={info.id}
          info={info}
          rowCount={view.rowCount}
          dialect={dialect}
          table={table ?? info.columns[0]?.table ?? null}
          sort={serverSort ? serverSort.sort : view.sort}
          onSort={(s) => void applySort(s)}
          version={view.version}
          onStatus={flash}
          onExport={() => exportMenu(window.innerWidth / 2, window.innerHeight / 2)}
          edit={edit}
        />
      )}
      {!info && run.outcomes.length > 0 && run.active === -1 && <Messages outcomes={run.outcomes} />}
      {!info && run.active >= 0 && outcome && !outcome.resultInfo && <Messages outcomes={[outcome]} />}
    </div>
  );

  // Table views: one toolbar on top, the row count and pager in a status bar below the grid.
  if (leading !== undefined) {
    return (
      <div className="results">
        <div className="results-bar toolbar">
          {leading}
          <span className="spacer" />
          {viewSwitch}
          {searchBox}
          {tools}
          {exportButton}
        </div>
        {body}
        <div className="status-bar">
          <span className="results-summary">{summaryText}</span>
          {status && <span className="results-status">{status}</span>}
          <span className="spacer" />
          {footer}
        </div>
        {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      </div>
    );
  }

  return (
    <div className="results">
      <div className="results-bar">
        <div className="results-tabs" role="tablist">
          {tabs.map(({ i, o }, n) => (
            <button
              key={i}
              role="tab"
              aria-selected={run.active === i}
              className={run.active === i ? "rtab active" : "rtab"}
              title={o.sql}
              onClick={() => setRunState(tabId, { active: i })}
            >
              {tabs.length > 1 ? `Result ${n + 1}` : "Result"}
            </button>
          ))}
          {run.outcomes.length > 0 && (
            <button
              role="tab"
              aria-selected={run.active === -1}
              className={run.active === -1 ? "rtab active" : "rtab"}
              onClick={() => setRunState(tabId, { active: -1 })}
            >
              Messages{errors > 0 && <span className="rtab-badge">{errors}</span>}
            </button>
          )}
        </div>
        <span className="results-summary">{summaryText}</span>
        {status && <span className="results-status">{status}</span>}
        <span className="spacer" />
        {run.running && onCancel && (
          <button className="btn small-btn" onClick={onCancel} title="Cancel (Ctrl+.)">
            Cancel
          </button>
        )}
        {viewSwitch}
        {searchBox}
        {!serverSort && run.lastRun && !run.running && (
          <button
            className="icon-btn"
            onClick={() => void rerunTab(tabId)}
            title={`Re-run: the same ${run.lastRun.statements.length === 1 ? "statement" : `${run.lastRun.statements.length} statements`} again, for fresh results`}
          >
            <RefreshIcon />
          </button>
        )}
        {exportButton}
      </div>
      {body}
      {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}

function Messages({ outcomes }: { outcomes: StatementOutcome[] }) {
  return (
    <div className="messages">
      {outcomes.map((o) => (
        <div key={o.index} className={o.error && !o.cancelled ? "msg error" : o.cancelled ? "msg cancelled" : "msg"}>
          <div className="msg-sql" title={o.sql}>
            {o.sql.length > 300 ? `${o.sql.slice(0, 300)}…` : o.sql}
          </div>
          <div className="msg-result">
            {o.error
              ? o.error
              : o.resultInfo
                ? `${o.resultInfo.rowCount.toLocaleString()} row${o.resultInfo.rowCount === 1 ? "" : "s"} returned${o.resultInfo.truncated ? " (first rows only)" : ""}`
                : `${(o.affectedRows ?? 0).toLocaleString()} row${o.affectedRows === 1 ? "" : "s"} affected${o.lastInsertId ? ` · last insert id ${o.lastInsertId}` : ""}`}
            {o.info && !o.error ? ` · ${o.info}` : ""}
            {o.warnings > 0 ? ` · ${o.warnings} warning${o.warnings === 1 ? "" : "s"}` : ""}
            <span className="msg-time">{formatElapsed(o.elapsedMs)}</span>
          </div>
        </div>
      ))}
    </div>
  );
}
