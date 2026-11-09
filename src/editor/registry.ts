/** The mounted SQL editors, so commands (Ctrl+Enter, format…) reach the active one. */
export interface EditorHandle {
  /** Selected text, or the statement under the cursor: split into statements. */
  currentStatements(): string[];
  allStatements(): string[];
  format(): string | null;
  focus(): void;
  insert(text: string): void;
  /** The whole editor text. */
  text(): string;
}

export const editors = new Map<string, EditorHandle>();

/** Text of editors that aren't on screen (kept by SqlEditor while their tab is hidden). */
export const hiddenEditorText = { get: (_tabId: string): string | undefined => undefined };

/** Called (often) when any editor's text changes. */
export const editorChanged = { notify: () => {} };
