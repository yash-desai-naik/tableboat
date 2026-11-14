import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { errorText, type SessionInfo } from "../lib/connections";
import { compactCount, metaApi, OBJECT_KINDS, objectKey, quoteIdent, type DbObject, type ObjectKind } from "../lib/meta";
import { fmtShortcut } from "../lib/platform";
import { getSettings, updateSettings, useSettings } from "../lib/settings";
import { uiActions } from "../lib/ui";
import { refreshAll } from "../lib/refresh";
import { openObject, setDatabase, useWorkspace } from "../lib/workspace";
import { openQuery } from "../editor/actions";
import { ChevronIcon, DatabaseIcon, FilterIcon, RefreshIcon } from "../shell/Icons";
import { Highlighted, KindIcon } from "../shell/KindIcon";
import { Menu, type MenuItem } from "../shell/Menu";

type Row =
  | { type: "section"; kind: ObjectKind; label: string; count: number; open: boolean }
  | { type: "item"; obj: DbObject; indices?: number[] };

/**
 * The object tree for the current database, VS Code style: type anywhere in the tree to
 * filter it (fuzzy, matched letters highlighted), arrows to move, Enter to open.
 */
export function Sidebar({ info }: { info: SessionInfo }) {
  const sessionId = info.sessionId;
  const engine = info.server.engine ?? info.connection.engine;
  const ws = useWorkspace();
  const settings = useSettings();
  const database = ws.database;

  const [objects, setObjects] = useState<DbObject[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [matches, setMatches] = useState<Map<string, number[]> | null>(null);
  const [collapsed, setCollapsed] = useState<Set<ObjectKind>>(new Set());
  const [cursor, setCursor] = useState(0);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const treeRef = useRef<HTMLDivElement>(null);

  const load = useCallback(
    async (refresh: boolean) => {
      if (!database) return;
      setLoading(true);
      setError(null);
      try {
        if (refresh) await metaApi.refresh(sessionId);
        setObjects(await metaApi.objects(sessionId, database, refresh));
      } catch (err) {
        setError(errorText(err));
        setObjects([]);
      } finally {
        setLoading(false);
      }
    },
    [sessionId, database],
  );

  useEffect(() => {
    void load(false);
  }, [load]);

  // Filtering runs in Rust (frizbee) so it stays fast with thousands of objects.
  useEffect(() => {
    if (!filter.trim() || !database) {
      setMatches(null);
      return;
    }
    let stale = false;
    metaApi
      .search(sessionId, filter, database, "name", 5000)
      .then((hits) => {
        if (stale) return;
        setMatches(new Map(hits.map((h) => [objectKey(h), h.indices])));
        setCursor(0);
      })
      .catch((err) => !stale && setError(errorText(err)));
    return () => {
      stale = true;
    };
  }, [filter, sessionId, database]);

  uiActions.refreshObjects = () => void load(true);
  uiActions.focusObjectFilter = () => {
    filterRef.current?.focus();
    filterRef.current?.select();
  };

  const visibleKinds = OBJECT_KINDS.filter((k) => settings.sidebarKinds.includes(k.id));

  const rows = useMemo<Row[]>(() => {
    const out: Row[] = [];
    for (const kind of visibleKinds) {
      const all = objects.filter((o) => o.kind === kind.id);
      const items = matches ? all.filter((o) => matches.has(objectKey(o))) : all;
      if (all.length === 0 || (matches && items.length === 0)) continue;
      // While filtering, every section with matches is shown open.
      const open = matches ? true : !collapsed.has(kind.id);
      const label = engine === "mongodb" && kind.id === "table" ? "Collections" : kind.plural;
      out.push({ type: "section", kind: kind.id, label, count: items.length, open });
      if (open) {
        for (const obj of items) out.push({ type: "item", obj, indices: matches?.get(objectKey(obj)) });
      }
    }
    return out;
  }, [objects, matches, collapsed, visibleKinds]);

  const activeKey = useMemo(() => {
    const tab = ws.tabs.find((t) => t.id === ws.activeId);
    return tab?.type === "object" ? objectKey(tab) : null;
  }, [ws.tabs, ws.activeId]);

  useEffect(() => {
    if (cursor >= rows.length) setCursor(Math.max(0, rows.length - 1));
  }, [rows.length, cursor]);

  useEffect(() => {
    treeRef.current?.querySelector(`[data-row="${cursor}"]`)?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  function toggleSection(kind: ObjectKind, open?: boolean) {
    setCollapsed((prev) => {
      const next = new Set(prev);
      const shouldOpen = open ?? next.has(kind);
      if (shouldOpen) next.delete(kind);
      else next.add(kind);
      return next;
    });
  }

  function copy(text: string) {
    void navigator.clipboard.writeText(text);
  }

  function itemMenu(obj: DbObject): MenuItem[] {
    const q = (s: string) => quoteIdent(s, engine);
    const items: MenuItem[] = [
      { label: "Open", hint: "Click", onSelect: () => openObject(obj) },
      { label: "Open in New Tab", hint: "Double-click", onSelect: () => openObject(obj, true) },
      "separator",
      { label: "Copy Name", onSelect: () => copy(obj.name) },
      { label: "Copy Qualified Name", onSelect: () => copy(`${q(obj.database)}.${q(obj.name)}`) },
    ];
    if (obj.kind === "table" || obj.kind === "view") {
      const select =
        engine === "mongodb"
          ? `db.getCollection(${JSON.stringify(obj.name)}).find({}).limit(${getSettings().pageSize})`
          : `SELECT * FROM ${q(obj.name)} LIMIT ${getSettings().pageSize};`;
      items.push(
        { label: "Open in SQL Editor", onSelect: () => openQuery(select) },
        { label: "Copy SELECT Statement", onSelect: () => copy(select) },
      );
    }
    items.push("separator", { label: "Refresh", hint: fmtShortcut("Ctrl+R"), onSelect: () => refreshAll() });
    return items;
  }

  function kindsMenu(x: number, y: number) {
    const current = getSettings().sidebarKinds;
    setMenu({
      x,
      y,
      items: OBJECT_KINDS.map((k) => ({
        label: `${current.includes(k.id) ? "✓  " : "    "}${k.plural}`,
        onSelect: () => {
          const next = current.includes(k.id) ? current.filter((c) => c !== k.id) : [...current, k.id];
          updateSettings({ sidebarKinds: OBJECT_KINDS.map((o) => o.id).filter((id) => next.includes(id)) });
        },
      })),
    });
  }

  function onTreeKey(e: React.KeyboardEvent) {
    const row = rows[cursor];
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setCursor((c) => Math.min(rows.length - 1, c + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (cursor === 0) filterRef.current?.focus();
      else setCursor((c) => Math.max(0, c - 1));
    } else if (e.key === "Home") {
      e.preventDefault();
      setCursor(0);
    } else if (e.key === "End") {
      e.preventDefault();
      setCursor(rows.length - 1);
    } else if (e.key === "ArrowLeft" && row?.type === "section") {
      e.preventDefault();
      toggleSection(row.kind, false);
    } else if (e.key === "ArrowRight" && row?.type === "section") {
      e.preventDefault();
      toggleSection(row.kind, true);
    } else if (e.key === "ArrowLeft" && row?.type === "item") {
      e.preventDefault();
      const sectionIndex = rows.slice(0, cursor).findLastIndex((r) => r.type === "section");
      if (sectionIndex >= 0) setCursor(sectionIndex);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      if (row?.type === "section") toggleSection(row.kind);
      else if (row?.type === "item") openObject(row.obj, e.key === "Enter");
    } else if (e.key === "Escape" && filter) {
      e.preventDefault();
      setFilter("");
    } else if (e.key === "ContextMenu" && row?.type === "item") {
      e.preventDefault();
      const el = treeRef.current?.querySelector(`[data-row="${cursor}"]`)?.getBoundingClientRect();
      if (el) setMenu({ x: el.left + 24, y: el.bottom, items: itemMenu(row.obj) });
    } else if (e.key.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey) {
      // Filter-on-type: any printable key starts filtering.
      e.preventDefault();
      setFilter((f) => f + e.key);
      filterRef.current?.focus();
    }
  }

  return (
    <div className="sidebar-inner">
      <div className="sidebar-head">
        <button
          className="db-switch"
          title={`Switch database (${fmtShortcut("Ctrl+K")})`}
          onClick={() => uiActions.openDatabasePicker()}
        >
          <DatabaseIcon size={14} />
          <span className="db-switch-name">{database ?? "Choose a database"}</span>
          <ChevronIcon open={true} size={10} />
        </button>
        <button className="icon-btn small" title="Object types to show" onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          kindsMenu(r.left, r.bottom + 4);
        }}>
          <FilterIcon size={14} />
        </button>
        <button
          className="icon-btn small"
          title={`Refresh databases, objects and the open tab (${fmtShortcut("Ctrl+R")})`}
          onClick={() => refreshAll()}
        >
          <RefreshIcon size={14} />
        </button>
      </div>

      <div className="sidebar-filter">
        <input
          ref={filterRef}
          className="input"
          placeholder="Filter objects  (^start  end$  'exact)"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown" || e.key === "Enter") {
              e.preventDefault();
              treeRef.current?.focus();
              if (e.key === "Enter") {
                const first = rows.find((r) => r.type === "item");
                if (first?.type === "item") openObject(first.obj, true);
              }
            } else if (e.key === "Escape") {
              e.preventDefault();
              if (filter) setFilter("");
              else treeRef.current?.focus();
            }
          }}
          spellCheck={false}
        />
      </div>

      <div
        ref={treeRef}
        className="tree"
        role="tree"
        aria-label="Database objects"
        tabIndex={0}
        onKeyDown={onTreeKey}
      >
        {!database && <div className="tree-note">Choose a database to see its objects.</div>}
        {database && loading && objects.length === 0 && <div className="tree-note">Loading…</div>}
        {error && <div className="tree-note error">{error}</div>}
        {database && !loading && !error && objects.length === 0 && (
          <div className="tree-note">This database has no tables, views or routines.</div>
        )}
        {matches && rows.length === 0 && <div className="tree-note">Nothing matches “{filter}”.</div>}

        {rows.map((row, i) =>
          row.type === "section" ? (
            <div
              key={`s-${row.kind}`}
              data-row={i}
              role="treeitem"
              aria-expanded={row.open}
              className={i === cursor ? "tree-section cursor" : "tree-section"}
              onClick={() => {
                setCursor(i);
                toggleSection(row.kind);
              }}
            >
              <ChevronIcon open={row.open} size={10} />
              <span>{row.label}</span>
              <span className="tree-count">{row.count}</span>
            </div>
          ) : (
            <div
              key={objectKey(row.obj)}
              data-row={i}
              role="treeitem"
              title={row.obj.comment ?? row.obj.name}
              className={[
                "tree-item",
                i === cursor ? "cursor" : "",
                activeKey === objectKey(row.obj) ? "active" : "",
              ].join(" ")}
              onClick={() => {
                setCursor(i);
                openObject(row.obj);
              }}
              onDoubleClick={() => openObject(row.obj, true)}
              onAuxClick={(e) => {
                if (e.button === 1) openObject(row.obj, true);
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                setCursor(i);
                setMenu({ x: e.clientX, y: e.clientY, items: itemMenu(row.obj) });
              }}
            >
              <KindIcon kind={row.obj.kind} />
              <span className="tree-name">
                <Highlighted text={row.obj.name} indices={row.indices} />
              </span>
              {row.obj.kind === "table" && row.obj.rows != null && row.obj.rows > 0 && (
                <span className="tree-meta" title="Estimated rows">
                  {compactCount(row.obj.rows)}
                </span>
              )}
              {row.obj.kind === "trigger" && row.obj.parent && <span className="tree-meta">{row.obj.parent}</span>}
            </div>
          ),
        )}
      </div>

      {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}

/** Used by the database picker: remember the choice and tell the sidebar. */
export function chooseDatabase(name: string) {
  setDatabase(name);
}
