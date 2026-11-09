/**
 * Actions owned by mounted components (sidebar, overlays) that commands and shortcuts
 * can trigger. Components replace these no-ops while they're mounted.
 */
export const uiActions = {
  openQuickOpen: () => {},
  openDatabasePicker: () => {},
  refreshObjects: () => {},
  focusObjectFilter: () => {},
  /** Connection switcher: "list", or straight to a new connection / URL. */
  openConnections: (_mode: "list" | "new" | "url" = "list") => {},
  openSettings: () => {},
};
