/**
 * Keyboard shortcuts are matched on the physical key (`event.code`), so they work the
 * same on every keyboard layout, and AltGr chords are never treated as shortcuts
 * (on German, Polish, etc. layouts AltGr+0 types "}" and must not toggle anything).
 */

const CODE_TO_KEY: Record<string, string> = {
  Equal: "=",
  NumpadAdd: "=",
  Minus: "-",
  NumpadSubtract: "-",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backslash: "\\",
  BracketLeft: "[",
  BracketRight: "]",
  Backquote: "`",
  Semicolon: ";",
  Quote: "'",
  Enter: "Enter",
  NumpadEnter: "Enter",
  Escape: "Escape",
  Tab: "Tab",
  Space: "Space",
  Backspace: "Backspace",
  Delete: "Delete",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
};

function codeToKey(code: string): string | null {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  if (code.startsWith("Numpad") && /\d$/.test(code)) return code.slice(-1);
  if (/^F\d{1,2}$/.test(code)) return code;
  return CODE_TO_KEY[code] ?? null;
}

/** "Ctrl+Shift+P" style name for a key event, or null if it can't be a shortcut. */
export function comboFromEvent(e: KeyboardEvent): string | null {
  if (e.getModifierState("AltGraph")) return null;
  const key = codeToKey(e.code);
  if (!key) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.shiftKey) parts.push("Shift");
  if (e.altKey) parts.push("Alt");
  parts.push(key);
  return parts.join("+");
}

/** Browser shortcuts WebView2 would otherwise act on (print, reload, find, view source...). */
const BROWSER_RESERVED = new Set([
  "Ctrl+P",
  "Ctrl+Shift+P",
  "Ctrl+R",
  "Ctrl+Shift+R",
  "F5",
  "Ctrl+F",
  "Ctrl+G",
  "Ctrl+Shift+G",
  "Ctrl+H",
  "Ctrl+J",
  "Ctrl+U",
  "Ctrl+S",
  "Ctrl+O",
  "Ctrl+N",
  "Ctrl+Shift+N",
  "F3",
  "F7",
  "F12",
  "Ctrl+Shift+I",
  "Ctrl+Shift+J",
  "Ctrl+Shift+C",
]);

export function isBrowserReserved(combo: string): boolean {
  return BROWSER_RESERVED.has(combo);
}

/** Split "Ctrl+Shift+P" into keycap labels. */
export function comboParts(combo: string): string[] {
  return combo.split("+").map((part) =>
    part === "Up" ? "↑" : part === "Down" ? "↓" : part === "Left" ? "←" : part === "Right" ? "→" : part,
  );
}
