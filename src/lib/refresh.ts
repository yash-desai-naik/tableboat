import { clearSchemaCache } from "../editor/schemaCache";
import { uiActions } from "./ui";

/** Open views (table data, definitions) that reload themselves on Refresh. */
const listeners = new Set<() => void>();

export function onRefresh(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Ctrl+R / F5: reload everything this window shows from the server: the database list and
 * object list (the sidebar clears the cached metadata), autocomplete names, and the open tab.
 */
export function refreshAll() {
  clearSchemaCache();
  uiActions.refreshObjects();
  for (const l of listeners) l();
}
