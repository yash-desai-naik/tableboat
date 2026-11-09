import { ask } from "@tauri-apps/plugin-dialog";
import { discardPending, hasPending, pendingCount } from "./edits";
import { closeAllTabs, closeOtherTabs, closeTab, getWorkspace } from "./workspace";

async function confirmDiscard(ids: string[]): Promise<boolean> {
  const dirty = ids.filter(hasPending);
  if (!dirty.length) return true;
  const n = dirty.reduce((a, id) => a + pendingCount(id), 0);
  const ok = await ask(`Discard ${n} unsaved change${n === 1 ? "" : "s"}?`, {
    title: "Unsaved changes",
    kind: "warning",
    okLabel: "Discard",
    cancelLabel: "Keep editing",
  });
  if (ok) dirty.forEach(discardPending);
  return ok;
}

export async function requestCloseTab(id: string) {
  if (await confirmDiscard([id])) closeTab(id);
}

export async function requestCloseOthers(id: string) {
  if (await confirmDiscard(getWorkspace().tabs.filter((t) => t.id !== id).map((t) => t.id))) closeOtherTabs(id);
}

export async function requestCloseAll() {
  if (await confirmDiscard(getWorkspace().tabs.map((t) => t.id))) closeAllTabs();
}
