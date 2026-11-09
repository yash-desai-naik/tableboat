import { useState } from "react";
import { activateTab, keepTab, useWorkspace } from "../lib/workspace";
import { hasPending, useAnyEditVersion } from "../lib/edits";
import { requestCloseAll, requestCloseOthers, requestCloseTab } from "../lib/tabs";
import { CloseSmallIcon, CommandIcon } from "../shell/Icons";
import { KindIcon } from "../shell/KindIcon";
import { Menu, type MenuItem } from "../shell/Menu";

/** Tabs: click to switch, middle-click or × to close, double-click a preview tab to keep it. */
export function TabStrip() {
  const ws = useWorkspace();
  // Redraw the unsaved-changes dots when edits are made, saved or discarded.
  useAnyEditVersion();
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);

  if (ws.tabs.length === 0) {
    return (
      <div className="tabs" role="tablist">
        <div className="tab active" role="tab" aria-selected="true">
          Overview
        </div>
      </div>
    );
  }

  return (
    <div className="tabs" role="tablist">
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
                { label: "Close", hint: "Ctrl+W", onSelect: () => void requestCloseTab(tab.id) },
                { label: "Close Others", onSelect: () => void requestCloseOthers(tab.id) },
                { label: "Close All", onSelect: () => void requestCloseAll() },
                ...(tab.preview ? (["separator", { label: "Keep Open", onSelect: () => keepTab(tab.id) }] as MenuItem[]) : []),
              ],
            });
          }}
        >
          {tab.type === "query" ? (
            <span className="kind-icon kind-query">
              <CommandIcon size={13} />
            </span>
          ) : (
            <KindIcon kind={tab.kind} size={13} />
          )}
          <span className="tab-title">{tab.type === "query" ? tab.title : tab.name}</span>
          {hasPending(tab.id) && <span className="tab-dirty" title="Unsaved changes" />}
          <button
            className="tab-close"
            title="Close (Ctrl+W)"
            onMouseDown={(e) => e.stopPropagation()}
            onClick={() => void requestCloseTab(tab.id)}
          >
            <CloseSmallIcon size={12} />
          </button>
        </div>
      ))}
      {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}
