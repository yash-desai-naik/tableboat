import { connectionActions } from "../connections/ConnectionList";
import { api } from "./connections";
import { OBJECT_KINDS } from "./meta";
import { disconnect, type SessionState } from "./session";
import { uiActions } from "./ui";
import { refreshAll } from "./refresh";
import { cancelActiveRun, focusActiveEditor, formatActive, openQuery, runAll, runCurrent } from "../editor/actions";
import { activateTabAt, cycleTab, getWorkspace, keepTab } from "./workspace";
import { requestCloseAll, requestCloseOthers, requestCloseTab } from "./tabs";
import { tableActions } from "./edits";
import { getSettings, updateSettings, type Density, type Theme } from "./settings";
import { shutdown as stopClaude } from "./claude";

export interface Command {
  id: string;
  category: string;
  title: string;
  /** Shortcuts; the first one is shown in the palette. */
  keys?: string[];
  /** For toggles and choices: shows a check mark in the palette. */
  checked?: boolean;
  /** Shortcut-only commands that would clutter the palette (e.g. Ctrl+1..9). */
  hidden?: boolean;
  run: () => void;
}

export interface CommandContext {
  openPalette: () => void;
  session: SessionState;
}

const ZOOM_STEPS = [0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];

function stepZoom(direction: 1 | -1) {
  const zoom = getSettings().uiZoom;
  const next =
    direction > 0
      ? ZOOM_STEPS.find((z) => z > zoom + 0.001)
      : [...ZOOM_STEPS].reverse().find((z) => z < zoom - 0.001);
  if (next !== undefined) updateSettings({ uiZoom: next });
}

function clampFont(size: number) {
  return Math.min(40, Math.max(8, size));
}

/** Every command in the app. Rebuilt when settings change so check marks stay current. */
export function buildCommands(ctx: CommandContext): Command[] {
  const s = getSettings();

  const theme = (value: Theme, title: string): Command => ({
    id: `theme.${value}`,
    category: "Theme",
    title,
    checked: s.theme === value,
    run: () => updateSettings({ theme: value }),
  });

  const density = (value: Density, title: string): Command => ({
    id: `density.${value}`,
    category: "Density",
    title,
    checked: s.density === value,
    run: () => updateSettings({ density: value }),
  });

  const connected = ctx.session.status === "open";
  const sessionInfo = ctx.session.status === "open" ? ctx.session.info : null;

  const activeTab = () => getWorkspace().activeId;
  const tableTab = () => {
    const id = getWorkspace().activeId;
    return id ? tableActions.get(id) : undefined;
  };
  const workspaceCommands: Command[] = connected
    ? [
        { id: "go.object", category: "Go", title: "Go to Object…", keys: ["Ctrl+P"], run: () => uiActions.openQuickOpen() },
        { id: "query.new", category: "Query", title: "New SQL Tab", keys: ["Ctrl+T", "Ctrl+E"], run: () => openQuery() },
        { id: "query.runCurrent", category: "Query", title: "Run Current Statement", keys: ["Ctrl+Enter"], run: runCurrent },
        { id: "query.runAll", category: "Query", title: "Run All Statements", keys: ["Ctrl+Shift+Enter"], run: runAll },
        { id: "query.cancel", category: "Query", title: "Cancel Running Query", keys: ["Ctrl+."], run: cancelActiveRun },
        { id: "query.format", category: "Query", title: "Format Selection or Current Statement", keys: ["Shift+Alt+F"], run: () => void formatActive() },
        { id: "query.formatAll", category: "Query", title: "Format All Statements", keys: ["Ctrl+Shift+Alt+F"], run: () => void formatActive("all") },
        { id: "query.focus", category: "Query", title: "Focus Editor", run: focusActiveEditor },
        { id: "query.stopOnError", category: "Query", title: "Stop at First Error", checked: s.stopOnError, run: () => updateSettings({ stopOnError: !getSettings().stopOnError }) },
        { id: "results.sortDesc", category: "Results", title: "Sort Descending on First Click", checked: s.sortDescendingFirst, run: () => updateSettings({ sortDescendingFirst: !getSettings().sortDescendingFirst }) },
        { id: "go.database", category: "Go", title: "Switch Database…", keys: ["Ctrl+K"], run: () => uiActions.openDatabasePicker() },
        { id: "sidebar.filter", category: "Sidebar", title: "Filter Objects", keys: ["Ctrl+Shift+F"], run: () => {
            if (!getSettings().showSidebar) updateSettings({ showSidebar: true });
            window.setTimeout(() => uiActions.focusObjectFilter(), 0);
          } },
        { id: "view.refresh", category: "View", title: "Refresh All (databases, objects, open tab)", keys: ["Ctrl+R", "F5"], run: () => refreshAll() },
        { id: "sidebar.refresh", category: "Sidebar", title: "Refresh Databases and Objects", run: () => uiActions.refreshObjects() },
        { id: "table.reload", category: "Tables", title: "Reload Table Data", run: () => tableTab()?.reload() },
        ...OBJECT_KINDS.map((k): Command => ({
          id: `sidebar.kind.${k.id}`,
          category: "Sidebar",
          title: `Show ${k.plural}`,
          checked: s.sidebarKinds.includes(k.id),
          run: () => {
            const cur = getSettings().sidebarKinds;
            const next = cur.includes(k.id) ? cur.filter((x) => x !== k.id) : [...cur, k.id];
            updateSettings({ sidebarKinds: OBJECT_KINDS.map((o) => o.id).filter((id) => next.includes(id)) });
          },
        })),
        { id: "tab.close", category: "Tabs", title: "Close Tab", keys: ["Ctrl+W", "Ctrl+F4"], run: () => { const id = activeTab(); if (id) void requestCloseTab(id); } },
        { id: "tab.closeOthers", category: "Tabs", title: "Close Other Tabs", run: () => { const id = activeTab(); if (id) void requestCloseOthers(id); } },
        { id: "tab.closeAll", category: "Tabs", title: "Close All Tabs", keys: ["Ctrl+Shift+W"], run: () => void requestCloseAll() },
        // Looked up when run: the open table tab registers these after it renders.
        { id: "edit.save", category: "Edit", title: "Save Changes", keys: ["Ctrl+S"], run: () => tableTab()?.save() },
        { id: "edit.discard", category: "Edit", title: "Discard Changes", keys: ["Ctrl+Shift+Delete"], run: () => tableTab()?.discard() },
        { id: "edit.preview", category: "Edit", title: "Preview Changes (SQL)", run: () => tableTab()?.preview() },
        { id: "edit.addRow", category: "Edit", title: "Add Row", keys: ["Ctrl+I"], run: () => tableTab()?.addRow() },
        { id: "edit.duplicate", category: "Edit", title: "Duplicate Selected Rows", run: () => tableTab()?.duplicate() },
        { id: "edit.delete", category: "Edit", title: "Delete Selected Rows", run: () => tableTab()?.deleteRows() },
        { id: "tab.keep", category: "Tabs", title: "Keep Tab Open", run: () => { const id = activeTab(); if (id) keepTab(id); } },
        { id: "tab.next", category: "Tabs", title: "Next Tab", keys: ["Ctrl+Tab", "Ctrl+PageDown"], run: () => cycleTab(1) },
        { id: "tab.previous", category: "Tabs", title: "Previous Tab", keys: ["Ctrl+Shift+Tab", "Ctrl+PageUp"], run: () => cycleTab(-1) },
        ...[1, 2, 3, 4, 5, 6, 7, 8, 9].map((n): Command => ({
          id: `tab.goto${n}`,
          category: "Tabs",
          title: `Go to Tab ${n}`,
          keys: [`Ctrl+${n}`],
          hidden: true,
          run: () => activateTabAt(n),
        })),
      ]
    : [];

  const switcherCommands: Command[] = [
    { id: "connection.open", category: "Connection", title: "Open Connection…", keys: ["Ctrl+Shift+K"], run: () => uiActions.openConnections("list") },
  ];

  const connectionCommands: Command[] = connected
    ? [
        { id: "connection.new", category: "Connection", title: "New Connection…", run: () => uiActions.openConnections("new") },
        { id: "connection.fromUrl", category: "Connection", title: "New Connection from URL…", run: () => uiActions.openConnections("url") },
        { id: "connection.disconnect", category: "Connection", title: "Disconnect", run: () => void disconnect() },
        {
          id: "connection.copyUrl",
          category: "Connection",
          title: "Copy Connection URL",
          run: async () => {
            if (sessionInfo) await navigator.clipboard.writeText(await api.toUrl(sessionInfo.connection.id, false));
          },
        },
      ]
    : [
        { id: "connection.new", category: "Connection", title: "New Connection…", run: () => connectionActions.newConnection() },
        { id: "connection.fromUrl", category: "Connection", title: "New Connection from URL…", run: () => connectionActions.importUrl() },
        { id: "connection.importFile", category: "Connection", title: "Import Connections from File…", run: () => connectionActions.importFile() },
        { id: "connection.exportFile", category: "Connection", title: "Export Connections to File…", run: () => connectionActions.exportFile() },
      ];

  return [
    {
      id: "palette.show",
      category: "Help",
      title: "Show All Commands",
      keys: ["Ctrl+Shift+P", "F1"],
      run: ctx.openPalette,
    },
    { id: "settings.open", category: "Preferences", title: "Open Settings", keys: ["Ctrl+,"], run: () => uiActions.openSettings() },
    {
      id: "settings.restoreTabs",
      category: "Preferences",
      title: "Reopen Tabs When Connecting",
      checked: s.restoreTabs,
      run: () => updateSettings({ restoreTabs: !getSettings().restoreTabs }),
    },
    {
      id: "settings.reopenLast",
      category: "Preferences",
      title: "Reconnect to Last Connection at Start-up",
      checked: s.reopenLastConnection,
      run: () => updateSettings({ reopenLastConnection: !getSettings().reopenLastConnection }),
    },
    ...[100, 300, 1000, 5000].map(
      (n): Command => ({
        id: `tables.pageSize.${n}`,
        category: "Tables",
        title: `Rows per Page: ${n.toLocaleString()}`,
        checked: s.pageSize === n,
        run: () => updateSettings({ pageSize: n }),
      }),
    ),
    ...[1000, 10_000, 100_000, 1_000_000].map(
      (n): Command => ({
        id: `query.maxRows.${n}`,
        category: "Query",
        title: `Row Limit: ${n.toLocaleString()}`,
        checked: s.maxRows === n,
        run: () => updateSettings({ maxRows: n }),
      }),
    ),
    ...workspaceCommands,
    ...switcherCommands,
    ...connectionCommands,
    ...(
      [
        ["", "Claude Code's default"],
        ["sonnet", "Sonnet (balanced)"],
        ["haiku", "Haiku (fastest, cheapest)"],
        ["opus", "Opus (most capable)"],
      ] as const
    ).map(([model, label]): Command => ({
      id: `claude.model.${model || "default"}`,
      category: "Claude",
      title: `Model: ${label}`,
      checked: s.claudeModel === model,
      run: () => {
        updateSettings({ claudeModel: model });
        // The running chat restarts with the new model on the next message (same conversation).
        void stopClaude().catch(() => {});
      },
    })),
    {
      id: "window.new",
      category: "Window",
      title: "New Window",
      keys: ["Ctrl+Shift+N"],
      run: () => void api.newWindow(),
    },
    {
      id: "view.toggleSidebar",
      category: "View",
      title: "Toggle Sidebar",
      keys: ["Ctrl+B"],
      checked: s.showSidebar,
      run: () => updateSettings({ showSidebar: !getSettings().showSidebar }),
    },
    {
      id: "view.toggleAiPane",
      category: "View",
      title: "Toggle Claude Pane",
      keys: ["Ctrl+Alt+B"],
      checked: s.showAiPane,
      run: () => updateSettings({ showAiPane: !getSettings().showAiPane }),
    },
    { id: "view.zoomIn", category: "View", title: "Zoom In", keys: ["Ctrl+=", "Ctrl+Shift+="], run: () => stepZoom(1) },
    { id: "view.zoomOut", category: "View", title: "Zoom Out", keys: ["Ctrl+-"], run: () => stepZoom(-1) },
    { id: "view.zoomReset", category: "View", title: "Reset Zoom", keys: ["Ctrl+0"], run: () => updateSettings({ uiZoom: 1 }) },
    theme("system", "Match Windows"),
    theme("light", "Light"),
    theme("dark", "Dark"),
    {
      id: "theme.toggle",
      category: "Theme",
      title: "Toggle Light / Dark",
      run: () => {
        const dark = document.documentElement.dataset.theme === "dark";
        updateSettings({ theme: dark ? "light" : "dark" });
      },
    },
    density("compact", "Compact"),
    density("default", "Default"),
    density("comfortable", "Comfortable"),
    {
      id: "editor.toggleWordWrap",
      category: "Editor",
      title: "Toggle Word Wrap",
      keys: ["Alt+Z"],
      checked: s.wordWrap,
      run: () => updateSettings({ wordWrap: !getSettings().wordWrap }),
    },
    {
      id: "editor.fontBigger",
      category: "Editor",
      title: "Increase Font Size",
      run: () => updateSettings({ editorFontSize: clampFont(getSettings().editorFontSize + 1) }),
    },
    {
      id: "editor.fontSmaller",
      category: "Editor",
      title: "Decrease Font Size",
      run: () => updateSettings({ editorFontSize: clampFont(getSettings().editorFontSize - 1) }),
    },
    {
      id: "results.fontBigger",
      category: "Results",
      title: "Increase Font Size",
      run: () => updateSettings({ gridFontSize: clampFont(getSettings().gridFontSize + 1) }),
    },
    {
      id: "results.fontSmaller",
      category: "Results",
      title: "Decrease Font Size",
      run: () => updateSettings({ gridFontSize: clampFont(getSettings().gridFontSize - 1) }),
    },
    {
      id: "developer.reload",
      category: "Developer",
      title: "Reload Window",
      run: () => window.location.reload(),
    },
  ];
}
