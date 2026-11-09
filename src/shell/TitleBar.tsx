import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { displayName, tagInfo } from "../lib/connections";
import type { SessionState } from "../lib/session";
import {
  AppMark,
  CloseGlyph,
  CommandIcon,
  MaximizeGlyph,
  MinimizeGlyph,
  PanelLeftIcon,
  PanelRightIcon,
  PlugIcon,
  RestoreGlyph,
} from "./Icons";

const appWindow = getCurrentWindow();

interface TitleBarProps {
  session: SessionState;
  sidebarOn: boolean;
  aiOn: boolean;
  onToggleSidebar: () => void;
  onToggleAi: () => void;
  onOpenPalette: () => void;
  onPillClick: (x: number, y: number) => void;
  onConnections: () => void;
}

/** Pick black or white text for a background colour, by WCAG contrast. */
function readableOn(hex: string): string {
  const n = parseInt(hex.replace("#", ""), 16);
  const channel = (v: number) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  const l = 0.2126 * channel((n >> 16) & 255) + 0.7152 * channel((n >> 8) & 255) + 0.0722 * channel(n & 255);
  return (1.05 / (l + 0.05)) >= ((l + 0.05) / 0.05) ? "#ffffff" : "#111111";
}

/**
 * Custom 40 px title bar. Empty areas and the connection pill drag the window
 * (and double-click maximises); Windows Aero Snap works by dragging to the edges.
 */
export function TitleBar(props: TitleBarProps) {
  const { session, sidebarOn, aiOn, onToggleSidebar, onToggleAi, onOpenPalette, onPillClick, onConnections } = props;
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const refresh = () => appWindow.isMaximized().then(setMaximized).catch(() => {});
    refresh();
    appWindow.onResized(refresh).then((fn) => (unlisten = fn));
    return () => unlisten?.();
  }, []);

  const open = session.status === "open" ? session.info : null;
  const color = open?.connection.color;
  const tag = open ? tagInfo(open.connection.tag) : null;

  useEffect(() => {
    void appWindow.setTitle(open ? `${displayName(open.connection)} – Tableboat` : "Tableboat");
  }, [open]);

  return (
    <header
      className="titlebar"
      data-tauri-drag-region
      style={color ? { borderBottom: `2px solid ${color}` } : undefined}
    >
      <div className="tb-group" data-tauri-drag-region>
        <span className="tb-mark" data-tauri-drag-region>
          <AppMark size={16} />
        </span>
        <button className="icon-btn" title="Toggle sidebar (Ctrl+B)" aria-pressed={sidebarOn} onClick={onToggleSidebar}>
          <PanelLeftIcon />
        </button>
        <button className="icon-btn" title="Open or create a connection (Ctrl+Shift+K)" onClick={onConnections}>
          <PlugIcon />
        </button>
      </div>

      <div className="tb-center" data-tauri-drag-region>
        {open ? (
          <button
            className="conn-pill live"
            style={{ ["--conn" as string]: color }}
            title="Connection details and actions"
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              onPillClick(r.left, r.bottom + 4);
            }}
          >
            {tag && (
              <span className="pill-tag" style={{ background: tag.color, color: readableOn(tag.color) }}>
                {tag.short}
              </span>
            )}
            <span className="conn-pill-text">
              <span className="pill-strong">{displayName(open.connection)}</span>
              <span className="pill-sep"> · </span>
              {open.server.product} {open.server.version}
              {open.server.database && (
                <>
                  <span className="pill-sep"> · </span>
                  {open.server.database}
                </>
              )}
              {open.server.ssh && <span className="pill-flag">SSH</span>}
              {open.server.tls && <span className="pill-flag">TLS</span>}
            </span>
          </button>
        ) : (
          <div className="conn-pill" data-tauri-drag-region>
            <span className="conn-pill-text" data-tauri-drag-region>
              {session.status === "connecting" ? `Connecting to ${session.name}…` : "No connection"}
            </span>
          </div>
        )}
      </div>

      <div className="tb-group" data-tauri-drag-region>
        <button className="icon-btn" title="Show all commands (Ctrl+Shift+P)" onClick={onOpenPalette}>
          <CommandIcon />
        </button>
        <button className="icon-btn" title="Toggle Claude pane (Ctrl+Alt+B)" aria-pressed={aiOn} onClick={onToggleAi}>
          <PanelRightIcon />
        </button>
      </div>

      <div className="caption-buttons">
        <button className="caption-btn" title="Minimize" onClick={() => appWindow.minimize()}>
          <MinimizeGlyph />
        </button>
        <button
          className="caption-btn"
          title={maximized ? "Restore Down" : "Maximize"}
          onClick={() => appWindow.toggleMaximize()}
        >
          {maximized ? <RestoreGlyph /> : <MaximizeGlyph />}
        </button>
        <button className="caption-btn close" title="Close" onClick={() => appWindow.close()}>
          <CloseGlyph />
        </button>
      </div>
    </header>
  );
}
