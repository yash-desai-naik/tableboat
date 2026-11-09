import { lazy, Suspense, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ConnectedView } from "./connections/ConnectedView";
import { ConnectionList } from "./connections/ConnectionList";
import { ConnectionSwitcher, type SwitcherMode } from "./connections/ConnectionSwitcher";
import { api, type SessionInfo } from "./lib/connections";
import { buildCommands, type Command } from "./lib/commands";
import { comboFromEvent, isBrowserReserved } from "./lib/keys";
import { disconnect, useSession } from "./lib/session";
import { updateSettings, useSettings } from "./lib/settings";
import { uiActions } from "./lib/ui";
import { resetWorkspace, useWorkspace } from "./lib/workspace";
import { flushWorkspace, rememberLastConnection, restoreFor, trackWorkspace } from "./lib/persist";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { SettingsPage } from "./settings/SettingsPage";
import { CommandPalette } from "./palette/CommandPalette";
import { DatabasePicker } from "./palette/DatabasePicker";
import { QuickOpen } from "./palette/QuickOpen";
import { Sidebar } from "./sidebar/Sidebar";
import { clearSchemaCache } from "./editor/schemaCache";

const QueryView = lazy(() => import("./workspace/QueryView").then((m) => ({ default: m.QueryView })));
const AiPane = lazy(() => import("./ai/AiPane").then((m) => ({ default: m.AiPane })));
const TableView = lazy(() => import("./workspace/TableView").then((m) => ({ default: m.TableView })));
import { TabStrip } from "./workspace/TabStrip";
import { DatabaseIcon } from "./shell/Icons";
import { shutdown as stopClaude } from "./lib/claude";
import { Menu, type MenuItem } from "./shell/Menu";
import { Splitter } from "./shell/Splitter";
import { TitleBar } from "./shell/TitleBar";

const SIDEBAR_DEFAULT = 260;
const AI_PANE_DEFAULT = 380;

export function App() {
  const settings = useSettings();
  const session = useSession();
  const ws = useWorkspace();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [overlay, setOverlay] = useState<null | "quickOpen" | "databases">(null);
  const [switcher, setSwitcher] = useState<SwitcherMode | null>(null);
  const [pillMenu, setPillMenu] = useState<{ x: number; y: number } | null>(null);
  const sessionId = session.status === "open" ? session.info.sessionId : null;

  // A new connection starts with a clean workspace on its default database; with no
  // default database, ask which one to show.
  useEffect(() => {
    clearSchemaCache();
    if (session.status !== "open") {
      trackWorkspace(null);
      resetWorkspace(null);
      return;
    }
    // SQLite's database (schema) name is always "main"; server.database holds the file name.
    const db = session.info.connection.engine === "sqlite" ? "main" : (session.info.server.database ?? null);
    const connectionId = session.info.connection.id;
    let cancelled = false;
    // Restore this connection's tabs first, then start saving changes to them.
    trackWorkspace(null);
    resetWorkspace(db);
    void restoreFor(connectionId, db).then((restored) => {
      if (cancelled) return;
      trackWorkspace(connectionId);
      if (!restored && !db) setOverlay("databases");
      if (getCurrentWindow().label === "main") void rememberLastConnection(connectionId);
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  uiActions.openQuickOpen = () => sessionId && setOverlay("quickOpen");
  uiActions.openDatabasePicker = () => sessionId && setOverlay("databases");
  uiActions.openConnections = (mode = "list") => setSwitcher(mode);
  const [settingsOpen, setSettingsOpen] = useState(false);
  uiActions.openSettings = () => setSettingsOpen(true);

  // Save open tabs before the window closes.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void getCurrentWindow()
      .onCloseRequested(async () => {
        await flushWorkspace();
      })
      .then((fn) => (unlisten = fn));
    return () => unlisten?.();
  }, []);

  const commands = useMemo(
    () => buildCommands({ openPalette: () => setPaletteOpen(true), session }),
    // Rebuild when settings or the connection change so the list stays current.
    [settings, session, ws],
  );

  const pillItems: MenuItem[] =
    session.status === "open"
      ? [
          {
            label: "Copy Connection URL",
            onSelect: () =>
              void api.toUrl(session.info.connection.id, false).then((u) => navigator.clipboard.writeText(u)),
          },
          { label: "Open in New Window", onSelect: () => void api.newWindow(session.info.connection.id) },
          "separator",
          { label: "Switch Connection…", hint: "Ctrl+Shift+K", onSelect: () => setSwitcher("list") },
          { label: "New Connection…", onSelect: () => setSwitcher("new") },
          "separator",
          { label: "Disconnect", onSelect: () => void disconnect() },
        ]
      : [];

  // Closing the Claude pane stops Claude Code (frees its memory); the chat resumes on the next message.
  useEffect(() => {
    if (!settings.showAiPane) void stopClaude().catch(() => {});
  }, [settings.showAiPane]);

  useThemeAndDensity(settings.theme, settings.density, settings.editorFontSize, settings.gridFontSize);
  useZoom(settings.uiZoom);
  useShortcuts(commands);
  useNoBrowserContextMenu();

  const toggleSidebar = () => updateSettings({ showSidebar: !settings.showSidebar });
  const toggleAi = () => updateSettings({ showAiPane: !settings.showAiPane });

  return (
    <div className="app">
      <TitleBar
        session={session}
        sidebarOn={settings.showSidebar}
        aiOn={settings.showAiPane}
        onToggleSidebar={toggleSidebar}
        onToggleAi={toggleAi}
        onOpenPalette={() => setPaletteOpen(true)}
        onPillClick={(x, y) => setPillMenu({ x, y })}
        onConnections={() => setSwitcher("list")}
      />

      <div className="workspace">
        {settings.showSidebar && (
          <>
            <aside className="sidebar" style={{ width: settings.sidebarWidth }}>
              {session.status === "open" ? (
                <Sidebar key={session.info.sessionId} info={session.info} />
              ) : (
                <>
                  <div className="pane-header">
                    <span className="pane-title">Explorer</span>
                  </div>
                  <div className="pane-empty">
                    <DatabaseIcon size={20} />
                    <p>No connection</p>
                    <p className="muted">Connect to see tables, views, functions and triggers.</p>
                  </div>
                </>
              )}
            </aside>
            <Splitter
              pane="left"
              label="Resize sidebar"
              width={settings.sidebarWidth}
              min={160}
              max={900}
              onResize={(w) => updateSettings({ sidebarWidth: w }, { debounceMs: 400 })}
              onReset={() => updateSettings({ sidebarWidth: SIDEBAR_DEFAULT })}
            />
          </>
        )}

        <main className="center">
          {session.status === "open" ? (
            <>
              <TabStrip />
              <CenterContent info={session.info} fallback={<ConnectedView info={session.info} />} />
            </>
          ) : (
            <>
              <div className="tabs" role="tablist">
                <div className="tab active" role="tab" aria-selected="true">
                  Connections
                </div>
              </div>
              {/* The list stays mounted while connecting, so a connection error can be shown on it. */}
              <ConnectionList />
            </>
          )}
        </main>

        {settings.showAiPane && (
          <>
            <Splitter
              pane="right"
              label="Resize Claude pane"
              width={settings.aiPaneWidth}
              min={260}
              max={1200}
              onResize={(w) => updateSettings({ aiPaneWidth: w }, { debounceMs: 400 })}
              onReset={() => updateSettings({ aiPaneWidth: AI_PANE_DEFAULT })}
            />
            <aside className="aipane" style={{ width: settings.aiPaneWidth }}>
              <Suspense fallback={null}>
                <AiPane onClose={toggleAi} />
              </Suspense>
            </aside>
          </>
        )}
      </div>

      {paletteOpen && <CommandPalette commands={commands} onClose={() => setPaletteOpen(false)} />}
      {overlay === "quickOpen" && sessionId && <QuickOpen sessionId={sessionId} onClose={() => setOverlay(null)} />}
      {settingsOpen && <SettingsPage onClose={() => setSettingsOpen(false)} />}
      {switcher && <ConnectionSwitcher initialMode={switcher} onClose={() => setSwitcher(null)} />}
      {overlay === "databases" && sessionId && (
        <DatabasePicker sessionId={sessionId} onClose={() => setOverlay(null)} />
      )}
      {pillMenu && pillItems.length > 0 && (
        <Menu x={pillMenu.x} y={pillMenu.y} items={pillItems} onClose={() => setPillMenu(null)} />
      )}
    </div>
  );
}

/** The active tab's content, or the connection overview when no tab is open. */
function CenterContent({ info, fallback }: { info: SessionInfo; fallback: ReactNode }) {
  const ws = useWorkspace();
  const tab = ws.tabs.find((t) => t.id === ws.activeId);
  if (!tab) return <>{fallback}</>;
  return (
    <Suspense fallback={<div className="results-note">Loading…</div>}>
      {tab.type === "query" ? <QueryView key={tab.id} info={info} tab={tab} /> : <TableView key={tab.id} info={info} tab={tab} />}
    </Suspense>
  );
}

function useThemeAndDensity(theme: string, density: string, editorFont: number, gridFont: number) {
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const resolved = theme === "system" ? (media.matches ? "dark" : "light") : theme;
      document.documentElement.dataset.theme = resolved;
    };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.density = density;
    root.style.setProperty("--editor-font-size", `${editorFont}px`);
    root.style.setProperty("--grid-font-size", `${gridFont}px`);
  }, [density, editorFont, gridFont]);
}

function useZoom(zoom: number) {
  useEffect(() => {
    getCurrentWebview()
      .setZoom(zoom)
      .catch((err) => console.error("Could not set zoom", err));
  }, [zoom]);
}

/** One keydown listener for the whole app; matches shortcuts against the command list. */
function useShortcuts(commands: Command[]) {
  const latest = useRef(commands);
  latest.current = commands;

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const combo = comboFromEvent(e);
      if (!combo) return;
      const command = latest.current.find((c) => c.keys?.includes(combo));
      if (command) {
        e.preventDefault();
        e.stopPropagation();
        command.run();
      } else if ((e.target as HTMLElement | null)?.closest?.(".cm-editor")) {
        // Let the SQL editor handle its own keys (find, select next occurrence, …).
        return;
      } else if (isBrowserReserved(combo)) {
        // Stop WebView2 from printing, reloading or opening its find bar.
        e.preventDefault();
      }
    };
    window.addEventListener("keydown", onKeyDown, { capture: true });
    return () => window.removeEventListener("keydown", onKeyDown, { capture: true });
  }, []);
}

/** Hide the browser's right-click menu, except in text fields where copy/paste is useful. */
function useNoBrowserContextMenu() {
  useEffect(() => {
    if (import.meta.env.DEV) return;
    const onContextMenu = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (target.closest("input, textarea, [contenteditable='true']")) return;
      e.preventDefault();
    };
    window.addEventListener("contextmenu", onContextMenu);
    return () => window.removeEventListener("contextmenu", onContextMenu);
  }, []);
}
