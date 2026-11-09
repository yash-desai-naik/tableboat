/**
 * Actions owned by mounted components (sidebar, overlays) that commands and shortcuts
 * can trigger. Components replace these no-ops while they're mounted.
 */
export const uiActions = {
  openQuickOpen: () => {},
  openDatabasePicker: () => {},
  refreshObjects: () => {},
  focusObjectFilter: () => {},
  /**
   * Connection switcher: "list" (open here), "window" (open in a new window), "edit" (the
   * connection open here), or straight to a new connection / URL.
   */
  openConnections: (_mode: "list" | "window" | "new" | "url" | "edit" = "list") => {},
  openSettings: () => {},
  openAbout: () => {},
};
