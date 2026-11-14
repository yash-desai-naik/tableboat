/**
 * Which desktop the app is running on, and the words and keys that differ.
 *
 * The UI runs in the system WebView on both platforms (WebView2 on Windows, WebKit on macOS),
 * so the user agent is the reliable signal without an extra Tauri plugin.
 */

export const IS_MAC = /Macintosh|Mac OS X/i.test(navigator.userAgent);

/** The OS, as people call it. */
export const OS_NAME = IS_MAC ? "macOS" : "Windows";

/** The primary shortcut modifier: Command on macOS, Ctrl on Windows. */
export function modPressed(e: { ctrlKey: boolean; metaKey: boolean }): boolean {
  return IS_MAC ? e.metaKey : e.ctrlKey;
}

/** Where saved passwords live. */
export const CREDENTIAL_STORE = IS_MAC ? "macOS Keychain" : "Windows Credential Manager";

/** How shortcuts are written here: "Ctrl+Shift+P" on Windows, "⌘⇧P" on macOS. */
export function fmtShortcut(combo: string): string {
  if (!IS_MAC) return combo;
  const parts = combo.split("+");
  // Ctrl+Tab cycles tabs on macOS too (⌘Tab is the system's app switcher).
  const control = parts.includes("Tab") || parts.includes("PageUp") || parts.includes("PageDown");
  return parts
    .map((part) => (part === "Ctrl" ? (control ? "⌃" : "⌘") : part === "Shift" ? "⇧" : part === "Alt" ? "⌥" : part))
    .join("");
}
