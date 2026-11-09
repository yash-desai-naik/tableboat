import { useSyncExternalStore } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errorText, type Connection, type SessionInfo } from "./connections";
import { flushWorkspace, rememberLastConnection } from "./persist";

/** The database session open in this window (one per window). */
export type SessionState =
  | { status: "none" }
  | { status: "connecting"; connectionId: string; name: string }
  | { status: "open"; info: SessionInfo };

let state: SessionState = { status: "none" };
const listeners = new Set<() => void>();

function set(next: SessionState) {
  state = next;
  for (const l of listeners) l();
}

export function useSession(): SessionState {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    () => state,
  );
}

export function getSession(): SessionState {
  return state;
}

/** Throws the backend's message; "PASSWORD_REQUIRED" means ask for the password and retry. */
export async function connect(connectionId: string, name: string, password?: string): Promise<SessionInfo> {
  const previous = state;
  if (previous.status === "open") await closeCurrent();
  set({ status: "connecting", connectionId, name });
  try {
    const info = await api.openSession(connectionId, password);
    set({ status: "open", info });
    return info;
  } catch (err) {
    set({ status: "none" });
    throw new Error(errorText(err));
  }
}

/** Fields that only change how a connection looks: editing them needs no reconnect. */
const COSMETIC = new Set(["name", "color", "tag", "createdAt", "updatedAt", "hasPassword", "hasSshPassword", "hasSshPassphrase"]);

/**
 * A saved connection was edited. When it's the one open in this window, show the new name,
 * colour and tag at once; returns true when other settings changed (host, user, SSL, SSH…),
 * which only take effect after reconnecting.
 */
export function applyEditedConnection(saved: Connection): boolean {
  if (state.status !== "open" || state.info.connection.id !== saved.id) return false;
  const before = state.info.connection;
  const key = (c: Connection) =>
    JSON.stringify(
      Object.entries(c)
        .filter(([k]) => !COSMETIC.has(k))
        .sort(([a], [b]) => a.localeCompare(b)),
    );
  const needsReconnect = key(before) !== key({ ...before, ...saved });
  set({ status: "open", info: { ...state.info, connection: { ...before, ...saved } } });
  return needsReconnect;
}

/** Disconnect on purpose: the next start-up shows the connection list instead of reconnecting. */
export async function disconnect(): Promise<void> {
  await flushWorkspace();
  await closeCurrent();
  if (getCurrentWindow().label === "main") void rememberLastConnection(null);
}

async function closeCurrent(): Promise<void> {
  const current = state;
  if (current.status !== "open") return;
  set({ status: "none" });
  try {
    await api.closeSession(current.info.sessionId);
  } catch (err) {
    console.error("Could not close the session", err);
  }
}
