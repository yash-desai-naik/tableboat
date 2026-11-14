import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { activateTab, keepTab, useWorkspace } from "../lib/workspace";
import { hasPending, useAnyEditVersion } from "../lib/edits";
import { requestCloseAll, requestCloseOthers, requestCloseTab } from "../lib/tabs";
import { fmtShortcut } from "../lib/platform";
import { CloseSmallIcon, CommandIcon } from "../shell/Icons";
import { KindIcon } from "../shell/KindIcon";
import { Menu, type MenuItem } from "../shell/Menu";

/**
 * Tabs: click to switch, middle-click or × to close, double-click a preview tab to keep it.
 * No scrollbar: the wheel scrolls sideways, the active tab scrolls into view, edges fade when
 * tabs are hidden, and a button lists every tab once they don't all fit.
 */
export function TabStrip() {
  const ws = useWorkspace();
  // Redraw the unsaved-changes dots when edits are made, saved or discarded.
  useAnyEditVersion();
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const strip = useRef<HTMLDivElement>(null);
  const [overflow, setOverflow] = useState({ left: false, right: false });

  const measure = () => {
    const el = strip.current;
    if (!el) return;
    const left = el.scrollLeft > 1;
    const right = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
    setOverflow((o) => (o.left === left && o.right === right ? o : { left, right }));
  };

  useLayoutEffect(measure, [ws.tabs.length]);
  useEffect(() => {
    const el = strip.current;
    if (!el) return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Keep the active tab in view.
  useEffect(() => {
    strip.current?.querySelector(".tab.active")?.scrollIntoView({ block: "nearest", inline: "nearest" });
    measure();
  }, [ws.activeId, ws.tabs.length]);

  if (ws.tabs.length === 0) {
    return (
      <div className="tabbar">
        <div className="tabs" role="tablist">
          <div className="tab active" role="tab" aria-selected="true">
            <span className="tab-title">Overview</span>
          </div>
        </div>
      </div>
    );
  }

  const title = (t: (typeof ws.tabs)[number]) => (t.type === "query" ? t.title : t.name);
  const icon = (t: (typeof ws.tabs)[number]) =>
    t.type === "query" ? (
      <span className="kind-icon kind-query">
        <CommandIcon size={13} />
      </span>
    ) : (
      <KindIcon kind={t.kind} size={13} />
    );

  return (
    <div className="tabbar">
      <div
        ref={strip}
        className={["tabs", overflow.left ? "fade-left" : "", overflow.right ? "fade-right" : ""].join(" ")}
        role="tablist"
        onScroll={measure}
        onWheel={(e) => {
          // A plain mouse wheel scrolls the tabs sideways.
          if (e.deltaY && !e.deltaX && strip.current) strip.current.scrollLeft += e.deltaY;
        }}
      >
        {ws.tabs.map((tab) => (
          <div
            key={tab.id}
            role="tab"
            aria-selected={tab.id === ws.activeId}
            className={["tab", tab.id === ws.activeId ? "active" : "", tab.preview ? "preview" : ""].join(" ")}
            title={tab.type === "query" ? tab.title : `${tab.database}.${tab.name}${tab.preview ? " (preview – double-click to keep)" : ""}`}
            onMouseDown={(e) => {
              if (e.button === 0) activateTab(tab.id);
            }}
            onDoubleClick={() => keepTab(tab.id)}
            onAuxClick={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                void requestCloseTab(tab.id);
              }
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              setMenu({
                x: e.clientX,
                y: e.clientY,
                items: [
                  { label: "Close", hint: fmtShortcut("Ctrl+W"), onSelect: () => void requestCloseTab(tab.id) },
                  { label: "Close Others", onSelect: () => void requestCloseOthers(tab.id) },
                  { label: "Close All", onSelect: () => void requestCloseAll() },
                  ...(tab.preview ? (["separator", { label: "Keep Open", onSelect: () => keepTab(tab.id) }] as MenuItem[]) : []),
                ],
              });
            }}
          >
            {icon(tab)}
            <span className="tab-title">{title(tab)}</span>
            {hasPending(tab.id) && <span className="tab-dirty" title="Unsaved changes" />}
            <button
              className="tab-close"
              title={`Close (${fmtShortcut("Ctrl+W")})`}
              onMouseDown={(e) => e.stopPropagation()}
              onClick={() => void requestCloseTab(tab.id)}
            >
              <CloseSmallIcon size={12} />
            </button>
          </div>
        ))}
      </div>
      {(overflow.left || overflow.right) && (
        <button
          className="icon-btn small tabs-all"
          title="All tabs"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            setMenu({
              x: r.right - 260,
              y: r.bottom + 4,
              items: ws.tabs.map((t) => ({
                label: `${t.id === ws.activeId ? "● " : ""}${title(t)}`,
                hint: t.type === "query" ? "query" : t.database,
                onSelect: () => activateTab(t.id),
              })),
            });
          }}
        >
          <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
            <path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>
      )}
      {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}
