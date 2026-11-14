import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  CompactSelection,
  DataEditor,
  GridCellKind,
  type DataEditorRef,
  type DrawCellCallback,
  type DrawHeaderCallback,
  type GridCell,
  type GridColumn,
  type GridSelection,
  type Item,
  type Theme,
} from "@glideapps/glide-data-grid";
import "@glideapps/glide-data-grid/dist/index.css";
import { errorText } from "../lib/connections";
import {
  addRow,
  deleteRows,
  duplicateRows,
  getPending,
  rowKey,
  setExisting,
  setInserted,
  type EditContext,
  type EditValue,
} from "../lib/edits";
import { queryApi, type CopyFormat, type Dialect, type ResultInfo } from "../lib/query";
import { getSettings, useSettings } from "../lib/settings";
import { Menu, type MenuItem } from "../shell/Menu";

const BLOCK = 200;
/** Longer values are cut for drawing; the full value is still copied and shown on open. */
const DRAW_LIMIT = 500;

export type SortState = { column: number; descending: boolean } | null;
type Row = (string | null)[];

/** Editing is on when the result is a table page with a usable primary key. */
export interface GridEditing {
  tabId: string;
  ctx: EditContext;
  /** Pending-changes version: redraw when it changes. */
  version: number;
  dialect: Dialect;
  engine?: string;
}

/** Selection helpers for the table toolbar (Duplicate, Delete…), by tab. */
export const gridHandles = new Map<string, { targets(): Promise<{ existing: Row[]; inserts: number[] }> }>();

interface Props {
  info: ResultInfo;
  /** Visible loaded rows (after quick search). */
  rowCount: number;
  dialect: Dialect;
  /** Table name used for "Copy as INSERT". */
  table: string | null;
  sort: SortState;
  onSort: (sort: SortState) => void;
  /** Changes whenever the visible rows change (sort, search, new data), to drop cached blocks. */
  version: number;
  onStatus?: (text: string) => void;
  onExport?: () => void;
  edit?: GridEditing;
}

function readTheme() {
  const css = getComputedStyle(document.documentElement);
  // Glide draws on a canvas, so it needs real colour strings: theme tokens are plain hex/rgba.
  const v = (name: string) => css.getPropertyValue(name).trim();
  const s = getSettings();
  const theme: Partial<Theme> = {
    accentColor: v("--accent"),
    accentFg: v("--accent-text"),
    accentLight: v("--accent-soft"),
    textDark: v("--text"),
    textMedium: v("--text-2"),
    textLight: v("--text-3"),
    textBubble: v("--text"),
    bgIconHeader: v("--text-3"),
    fgIconHeader: v("--bg"),
    textHeader: v("--text"),
    textHeaderSelected: v("--accent-text"),
    bgCell: v("--bg"),
    bgCellMedium: v("--bg-chrome"),
    bgHeader: v("--bg-chrome"),
    bgHeaderHasFocus: v("--bg-active"),
    bgHeaderHovered: v("--bg-hover"),
    bgBubble: v("--bg-elevated"),
    bgBubbleSelected: v("--bg-active"),
    bgSearchResult: v("--search-match"),
    borderColor: v("--grid-line"),
    horizontalBorderColor: "transparent",
    headerBottomBorderColor: v("--border-strong"),
    drilldownBorder: v("--border"),
    linkColor: v("--accent"),
    cellHorizontalPadding: 8,
    cellVerticalPadding: 3,
    headerFontStyle: `600 ${s.gridFontSize}px`,
    baseFontStyle: `${s.gridFontSize}px`,
    markerFontStyle: "11px",
    fontFamily: v("--font-mono"),
    editorFontSize: `${s.gridFontSize}px`,
    lineHeight: 1.4,
  };
  return {
    theme,
    zebra: v("--zebra"),
    muted: v("--text-3"),
    // Selected column header: accent fill with accent text on it.
    headerSelected: v("--accent-fill"),
    // Pending changes: amber edited cells, green new rows, red deleted rows.
    changed: v("--edit-changed"),
    added: v("--edit-added"),
    deleted: v("--edit-deleted"),
  };
}

function initialWidth(name: string, type: string) {
  // MongoDB ObjectIds are shown in full: ObjectId('0123456789abcdef01234567').
  if (type === "objectId") return 270;
  return Math.round(Math.min(320, Math.max(72, Math.max(name.length * 7.6, type.length * 6.2) + 34)));
}

function valueCell(v: EditValue, align: "right" | undefined, writable: boolean, bg?: string, muted?: string): GridCell {
  const themeOverride = bg || muted ? { ...(bg ? { bgCell: bg } : {}), ...(muted ? { textDark: muted } : {}) } : undefined;
  if (v.kind !== "text") {
    // "" data + NULL/DEFAULT display marks it apart from the text "NULL"; drawn italic below.
    return {
      kind: GridCellKind.Text,
      data: "",
      displayData: v.kind === "null" ? "NULL" : "DEFAULT",
      allowOverlay: writable,
      readonly: !writable,
      themeOverride,
    };
  }
  const value = v.value;
  return {
    kind: GridCellKind.Text,
    data: value,
    displayData: value.length > DRAW_LIMIT ? `${value.slice(0, DRAW_LIMIT)}…` : value.replace(/\r?\n/g, " ↵ "),
    allowOverlay: true,
    readonly: !writable,
    contentAlign: align,
    themeOverride,
  };
}

/** Results grid. Rows are fetched from Rust in blocks as they scroll into view. */
export function ResultGrid({ info, rowCount, dialect, table, sort, onSort, version, onStatus, onExport, edit }: Props) {
  const settings = useSettings();
  const gridRef = useRef<DataEditorRef>(null);
  const cache = useRef(new Map<number, Row[]>());
  const pending = useRef(new Set<number>());
  const [, setTick] = useState(0);
  const [widths, setWidths] = useState<number[]>(() => info.columns.map((c) => initialWidth(c.name, c.typeName)));
  const [selection, setSelection] = useState<GridSelection>({
    rows: CompactSelection.empty(),
    columns: CompactSelection.empty(),
  });
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const [themeKey, setThemeKey] = useState(0);
  // Helpers read the selection through a ref, so a right-click can update it and act on it at once.
  const selRef = useRef(selection);
  selRef.current = selection;
  const select = (next: GridSelection) => {
    selRef.current = next;
    setSelection(next);
  };
  const look = useMemo(() => readTheme(), [themeKey, settings.theme, settings.gridFontSize, settings.density]);

  const inserts = edit ? getPending(edit.tabId).inserts.length : 0;
  const totalRows = rowCount + inserts;

  // Follow theme switches (system light/dark, another colour theme, a Settings preview).
  useEffect(() => {
    const obs = new MutationObserver(() => setThemeKey((k) => k + 1));
    obs.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-color-theme", "data-density"] });
    return () => obs.disconnect();
  }, []);

  // New data or order: drop cached rows.
  useEffect(() => {
    cache.current.clear();
    pending.current.clear();
    setTick((t) => t + 1);
  }, [info.id, version]);

  useEffect(() => {
    setWidths(info.columns.map((c) => initialWidth(c.name, c.typeName)));
    setSelection({ rows: CompactSelection.empty(), columns: CompactSelection.empty() });
  }, [info.id, info.columns]);

  const fetchBlock = useCallback(
    (block: number) => {
      if (pending.current.has(block)) return;
      pending.current.add(block);
      const resultId = info.id;
      void queryApi
        .rows(resultId, block * BLOCK, BLOCK)
        .then((rows) => {
          if (resultId !== info.id) return;
          cache.current.set(block, rows);
          pending.current.delete(block);
          const cells: { cell: Item }[] = [];
          const end = Math.min(rowCount, (block + 1) * BLOCK);
          for (let r = block * BLOCK; r < end; r++) {
            for (let c = 0; c < info.columns.length; c++) cells.push({ cell: [c, r] });
          }
          gridRef.current?.updateCells(cells.length > 20000 ? cells.slice(0, 20000) : cells);
        })
        .catch((err) => {
          pending.current.delete(block);
          onStatus?.(errorText(err));
        });
    },
    [info.id, info.columns.length, rowCount, onStatus],
  );

  const cachedRow = useCallback((row: number): Row | undefined => {
    const block = Math.floor(row / BLOCK);
    return cache.current.get(block)?.[row - block * BLOCK];
  }, []);

  const getCellContent = useCallback(
    ([col, row]: Item): GridCell => {
      const meta = info.columns[col];
      const align = meta?.numeric ? ("right" as const) : undefined;
      const writable = !!edit && !edit.ctx.columns[col]?.generated;
      const p = edit ? getPending(edit.tabId) : null;

      // New rows come after the loaded ones.
      if (p && row >= rowCount) {
        const v = p.inserts[row - rowCount]?.values[col] ?? ({ kind: "default" } as EditValue);
        return valueCell(v, align, writable, look.added);
      }

      const block = Math.floor(row / BLOCK);
      const rows = cache.current.get(block);
      if (!rows) {
        fetchBlock(block);
        return { kind: GridCellKind.Loading, allowOverlay: false };
      }
      const loaded = rows[row - block * BLOCK];
      const original = loaded?.[col] ?? null;
      const base: EditValue = original === null ? { kind: "null" } : { kind: "text", value: original };
      if (p && edit && loaded) {
        const key = rowKey(loaded, edit.ctx.keyIdx);
        if (p.deletes.has(key)) return valueCell(base, align, false, look.deleted, look.muted);
        const change = p.updates.get(key)?.changes.get(col);
        if (change) return valueCell(change, align, writable, look.changed);
      }
      return valueCell(base, align, writable);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [info, fetchBlock, version, edit?.version, rowCount, look],
  );

  const columns = useMemo<GridColumn[]>(
    () => info.columns.map((c, i) => ({ id: String(i), title: c.name, width: widths[i] ?? 120 })),
    [info.columns, widths],
  );

  const drawHeader = useCallback<DrawHeaderCallback>(
    (args) => {
      const { ctx, rect, columnIndex, theme: t, isHovered, isSelected } = args;
      const meta = info.columns[columnIndex];
      if (!meta) return;
      ctx.fillStyle = isSelected ? look.headerSelected : isHovered ? t.bgHeaderHovered : t.bgHeader;
      ctx.fillRect(rect.x, rect.y, rect.width, rect.height);
      const pad = 8;
      const mid = rect.y + rect.height / 2;
      let right = rect.x + rect.width - pad;
      ctx.textBaseline = "middle";
      if (sort?.column === columnIndex) {
        ctx.fillStyle = isSelected ? t.textHeaderSelected : t.accentColor;
        ctx.font = `700 11px ${t.fontFamily}`;
        ctx.textAlign = "right";
        ctx.fillText(sort.descending ? "↓" : "↑", right, mid);
        right -= 14;
      }
      const tableCol = edit?.ctx.columns[columnIndex];
      const type =
        meta.typeName +
        (meta.primaryKey || tableCol?.primaryKey ? " ·PK" : "") +
        (tableCol?.generated ? " ·gen" : "");
      ctx.font = `11px ${t.fontFamily}`;
      const typeWidth = ctx.measureText(type).width;
      const nameSpace = right - (rect.x + pad);
      ctx.font = `600 12px ${t.fontFamily}`;
      const nameWidth = ctx.measureText(meta.name).width;
      if (type && nameWidth + typeWidth + 12 <= nameSpace) {
        ctx.font = `11px ${t.fontFamily}`;
        ctx.fillStyle = isSelected ? t.textHeaderSelected : t.textLight;
        const alpha = ctx.globalAlpha;
        if (isSelected) ctx.globalAlpha = alpha * 0.8;
        ctx.textAlign = "right";
        ctx.fillText(type, right, mid);
        ctx.globalAlpha = alpha;
      }
      ctx.font = `600 12px ${t.fontFamily}`;
      ctx.fillStyle = isSelected ? t.textHeaderSelected : t.textHeader;
      ctx.textAlign = "left";
      let name = meta.name;
      if (nameWidth > nameSpace) {
        while (name.length > 1 && ctx.measureText(`${name}…`).width > nameSpace) name = name.slice(0, -1);
        name += "…";
      }
      ctx.fillText(name, rect.x + pad, mid);
    },
    [info.columns, sort, edit?.ctx, look],
  );

  const drawCell = useCallback<DrawCellCallback>(
    (args, drawContent) => {
      const { cell, ctx, rect, theme: t } = args;
      if (cell.kind === GridCellKind.Text && cell.data === "" && (cell.displayData === "NULL" || cell.displayData === "DEFAULT")) {
        ctx.font = `italic ${t.baseFontStyle} ${t.fontFamily}`;
        ctx.fillStyle = look.muted;
        ctx.textBaseline = "middle";
        ctx.textAlign = "left";
        ctx.fillText(cell.displayData, rect.x + t.cellHorizontalPadding, rect.y + rect.height / 2);
        return;
      }
      drawContent();
    },
    [look.muted],
  );

  // ----- selection -----

  function selectedRowIndices(): number[] {
    const rows = selRef.current.rows.toArray();
    if (rows.length) return rows;
    const cur = selRef.current.current;
    if (!cur) return [];
    const set = new Set<number>();
    for (const r of [cur.range, ...cur.rangeStack]) for (let y = r.y; y < r.y + r.height; y++) set.add(y);
    return [...set].sort((a, b) => a - b);
  }

  function selectedCells(): Item[] {
    const cur = selRef.current.current;
    if (!cur) return [];
    const out: Item[] = [];
    for (const r of [cur.range, ...cur.rangeStack])
      for (let x = r.x; x < r.x + r.width; x++) for (let y = r.y; y < r.y + r.height; y++) out.push([x, y]);
    return out;
  }

  /** Loaded values of the selected rows (fetching any not yet drawn), and selected new rows. */
  async function targets(): Promise<{ existing: Row[]; inserts: number[] }> {
    const existing: Row[] = [];
    const insertsSel: number[] = [];
    for (const r of selectedRowIndices()) {
      if (r >= rowCount) {
        insertsSel.push(r - rowCount);
        continue;
      }
      const row = cachedRow(r) ?? (await queryApi.rows(info.id, r, 1))[0];
      if (row) existing.push(row);
    }
    return { existing, inserts: insertsSel };
  }

  useEffect(() => {
    if (!edit) return;
    gridHandles.set(edit.tabId, { targets });
    return () => {
      gridHandles.delete(edit.tabId);
    };
  });

  // New rows (Add Row, Duplicate) go after the loaded ones: scroll to them and select them,
  // so they're visible straight away instead of below the last loaded row.
  const knownInserts = useRef(inserts);
  useEffect(() => {
    const before = knownInserts.current;
    knownInserts.current = inserts;
    if (inserts <= before) return;
    const first = rowCount + before;
    const last = rowCount + inserts - 1;
    select({
      rows: CompactSelection.fromSingleSelection([first, last + 1]),
      columns: CompactSelection.empty(),
      current: { cell: [0, first], range: { x: 0, y: first, width: 1, height: 1 }, rangeStack: [] },
    });
    // Wait one frame so the grid knows about the new rows before scrolling.
    requestAnimationFrame(() => gridRef.current?.scrollTo(0, last, "vertical", 0, 0, { vAlign: "end" }));
    const n = inserts - before;
    onStatus?.(`${n} new row${n === 1 ? "" : "s"} added at the bottom – Save to keep ${n === 1 ? "it" : "them"}`);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inserts]);

  function selectionArgs(): { ranges: [number, number][] | null; columns: number[] | null } | null {
    const rows = selRef.current.rows.toArray().filter((r) => r < rowCount);
    if (rows.length) return { ranges: toRanges(rows), columns: null };
    const cols = selRef.current.columns.toArray();
    if (cols.length) return { ranges: null, columns: cols };
    const cur = selRef.current.current;
    if (!cur) return null;
    const colSet = new Set<number>();
    const rowList: number[] = [];
    for (const r of [cur.range, ...cur.rangeStack]) {
      for (let c = r.x; c < r.x + r.width; c++) colSet.add(c);
      for (let y = r.y; y < Math.min(rowCount, r.y + r.height); y++) rowList.push(y);
    }
    if (!rowList.length) return null;
    return { ranges: toRanges([...new Set(rowList)].sort((a, b) => a - b)), columns: [...colSet].sort((a, b) => a - b) };
  }

  async function copy(format: CopyFormat, whole = false) {
    const sel = whole ? { ranges: null, columns: null } : selectionArgs();
    if (!sel) return;
    try {
      const text = await queryApi.copy({ resultId: info.id, ...sel, format, dialect, table });
      await writeClipboard(text);
      const n = sel.ranges ? sel.ranges.reduce((a, [s, e]) => a + (e - s), 0) : rowCount;
      onStatus?.(`Copied ${n.toLocaleString()} row${n === 1 ? "" : "s"}`);
    } catch (err) {
      onStatus?.(errorText(err));
    }
  }

  // ----- editing -----

  function setCells(value: EditValue) {
    if (!edit) return;
    for (const [col, row] of selectedCells()) {
      if (edit.ctx.columns[col]?.generated) continue;
      if (row >= rowCount) setInserted(edit.tabId, row - rowCount, col, value);
      else {
        const loaded = cachedRow(row);
        if (loaded) setExisting(edit.tabId, edit.ctx, loaded, col, value);
      }
    }
  }

  async function duplicate() {
    if (!edit) return;
    const t = await targets();
    if (!t.existing.length && !t.inserts.length) {
      onStatus?.("Select one or more rows to duplicate.");
      return;
    }
    duplicateRows(edit.tabId, edit.ctx, t.existing, t.inserts);
  }

  async function remove() {
    if (!edit) return;
    const t = await targets();
    if (!t.existing.length && !t.inserts.length) return;
    deleteRows(edit.tabId, edit.ctx, t.existing, t.inserts);
    select({ rows: CompactSelection.empty(), columns: CompactSelection.empty() });
  }

  function contextMenu(x: number, y: number) {
    const hasSel = selectionArgs() !== null;
    const editItems: MenuItem[] = edit
      ? [
          { label: "Set NULL", onSelect: () => setCells({ kind: "null" }) },
          { label: "Set Empty", onSelect: () => setCells({ kind: "text", value: "" }) },
          ...(edit.engine === "mongodb"
            ? ([{ label: "Remove Field", onSelect: () => setCells({ kind: "default" }) }] as MenuItem[])
            : edit.dialect === "mysql"
              ? ([{ label: "Set DEFAULT", onSelect: () => setCells({ kind: "default" }) }] as MenuItem[])
              : []),
          "separator",
          { label: "Add Row", hint: "Ctrl+I", onSelect: () => addRow(edit.tabId, edit.ctx) },
          { label: "Duplicate Row(s)", hint: "Ctrl+D", onSelect: () => void duplicate() },
          { label: "Delete Row(s)", hint: "Delete", danger: true, onSelect: () => void remove() },
          "separator",
        ]
      : [];
    setMenu({
      x,
      y,
      items: [
        ...editItems,
        { label: "Copy", hint: "Ctrl+C", disabled: !hasSel, onSelect: () => void copy("tsv") },
        { label: "Copy with Headers", hint: "Ctrl+Shift+C", disabled: !hasSel, onSelect: () => void copy("tsv-header") },
        "separator",
        { label: "Copy as CSV", disabled: !hasSel, onSelect: () => void copy("csv-header") },
        { label: "Copy as JSON", disabled: !hasSel, onSelect: () => void copy("json") },
        { label: "Copy as Markdown", disabled: !hasSel, onSelect: () => void copy("markdown") },
        { label: "Copy as SQL INSERT", disabled: !hasSel, onSelect: () => void copy("insert") },
        "separator",
        { label: "Copy All Rows as CSV", onSelect: () => void copy("csv-header", true) },
        { label: "Copy All Rows as JSON", onSelect: () => void copy("json", true) },
        ...(onExport ? (["separator", { label: "Export…", onSelect: onExport }] as MenuItem[]) : []),
      ],
    });
  }

  const rowHeight = settings.density === "compact" ? 22 : settings.density === "comfortable" ? 28 : 24;

  return (
    <div
      className="result-grid"
      onKeyDownCapture={(e) => {
        // Keys typed into the cell editor are not grid shortcuts.
        if ((e.target as HTMLElement).closest("textarea, input")) return;
        if (e.ctrlKey && !e.altKey && e.code === "KeyC") {
          e.preventDefault();
          e.stopPropagation();
          void copy(e.shiftKey ? "tsv-header" : "tsv");
        } else if (edit && e.ctrlKey && !e.altKey && !e.shiftKey && e.code === "KeyD") {
          e.preventDefault();
          e.stopPropagation();
          void duplicate();
        } else if (edit && !e.ctrlKey && !e.altKey && e.key === "Delete") {
          e.preventDefault();
          e.stopPropagation();
          void remove();
        }
      }}
    >
      <DataEditor
        ref={gridRef}
        columns={columns}
        rows={totalRows}
        getCellContent={getCellContent}
        theme={look.theme}
        rowHeight={rowHeight}
        headerHeight={rowHeight + 4}
        // Clicking a row number selects the row (Ctrl/Shift-click for several), as in TablePlus.
        rowMarkers="clickable-number"
        smoothScrollX
        smoothScrollY
        width="100%"
        height="100%"
        rangeSelect="multi-rect"
        columnSelect="multi"
        rowSelect="multi"
        gridSelection={selection}
        onGridSelectionChange={select}
        getRowThemeOverride={(row) => (row % 2 === 1 && row < rowCount ? { bgCell: look.zebra } : undefined)}
        drawHeader={drawHeader}
        drawCell={drawCell}
        keybindings={{
          copy: false,
          paste: false,
          cut: false,
          delete: false,
          search: false,
          selectAll: true,
          selectColumn: true,
          selectRow: true,
        }}
        onCellEdited={
          edit
            ? ([col, row], value) => {
                if (value.kind !== GridCellKind.Text) return;
                const v: EditValue = { kind: "text", value: value.data };
                if (row >= rowCount) setInserted(edit.tabId, row - rowCount, col, v);
                else {
                  const loaded = cachedRow(row);
                  if (loaded) setExisting(edit.tabId, edit.ctx, loaded, col, v);
                }
              }
            : undefined
        }
        onColumnResize={(_col, size, index) =>
          setWidths((w) => {
            const next = [...w];
            next[index] = size;
            return next;
          })
        }
        onHeaderClicked={(col) => {
          // Cycle: first direction → other direction → original order.
          const first = settings.sortDescendingFirst;
          if (sort?.column !== col) onSort({ column: col, descending: first });
          else if (sort.descending === first) onSort({ column: col, descending: !first });
          else onSort(null);
        }}
        onHeaderContextMenu={(_col, e) => {
          e.preventDefault();
          contextMenu(e.bounds.x + e.localEventX, e.bounds.y + e.localEventY);
        }}
        onCellContextMenu={(cell, e) => {
          e.preventDefault();
          const inSel =
            selection.current &&
            [selection.current.range, ...selection.current.rangeStack].some(
              (r) => cell[0] >= r.x && cell[0] < r.x + r.width && cell[1] >= r.y && cell[1] < r.y + r.height,
            );
          if (!inSel && !selection.rows.hasIndex(cell[1])) {
            select({
              current: { cell, range: { x: cell[0], y: cell[1], width: 1, height: 1 }, rangeStack: [] },
              rows: CompactSelection.empty(),
              columns: CompactSelection.empty(),
            });
          }
          contextMenu(e.bounds.x + e.localEventX, e.bounds.y + e.localEventY);
        }}
        minColumnWidth={48}
        maxColumnWidth={1200}
      />
      {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}

function toRanges(sorted: number[]): [number, number][] {
  const out: [number, number][] = [];
  for (const n of sorted) {
    const last = out[out.length - 1];
    if (last && last[1] === n) last[1] = n + 1;
    else out.push([n, n + 1]);
  }
  return out;
}

/** Windows' clipboard can be briefly locked by another app; retry a few times. */
async function writeClipboard(text: string) {
  for (let attempt = 0; ; attempt++) {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch (err) {
      if (attempt >= 4) throw err;
      await new Promise((r) => setTimeout(r, 60 * (attempt + 1)));
    }
  }
}
