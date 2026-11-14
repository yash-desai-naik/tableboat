import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import type { Extension } from "@codemirror/state";

/** Token colours come from the theme (`--syn-*` in styles.css), so one style serves every theme. */
const syntax = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.modifier, t.controlKeyword], color: "var(--syn-keyword)" },
  { tag: [t.typeName, t.className], color: "var(--syn-type)" },
  { tag: [t.standard(t.name), t.function(t.variableName)], color: "var(--syn-function)" },
  { tag: [t.string, t.special(t.string)], color: "var(--syn-string)" },
  { tag: [t.number], color: "var(--syn-number)" },
  { tag: [t.bool, t.null, t.atom], color: "var(--syn-literal)" },
  { tag: [t.comment, t.lineComment, t.blockComment], color: "var(--syn-comment)" },
  { tag: [t.variableName, t.propertyName, t.special(t.name)], color: "var(--syn-property)" },
  { tag: [t.name], color: "var(--syn-operator)" },
  { tag: [t.operator, t.punctuation, t.bracket, t.separator], color: "var(--syn-operator)" },
  { tag: t.invalid, color: "var(--syn-invalid)" },
]);

/** Editor chrome. `dark` only tells CodeMirror which base styles to use; colours are theme tokens. */
function chrome(dark: boolean): Extension {
  return EditorView.theme(
    {
      "&": {
        height: "100%",
        backgroundColor: "var(--bg)",
        color: "var(--syn-operator)", // plain identifiers: the syntax foreground, as in VS Code
        fontSize: "var(--editor-font-size)",
      },
      ".cm-scroller": {
        fontFamily: "var(--font-mono)",
        lineHeight: "1.55",
      },
      ".cm-content": { caretColor: "var(--accent)", padding: "6px 0" },
      ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)", borderLeftWidth: "2px" },
      "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, ::selection":
        { backgroundColor: "var(--selection)" },
      ".cm-gutters": {
        backgroundColor: "var(--bg)",
        color: "var(--editor-gutter)",
        border: "none",
      },
      ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--editor-gutter-active)" },
      ".cm-activeLine": { backgroundColor: "transparent" },
      // The statement Ctrl+Enter will run.
      ".cm-current-stmt": { backgroundColor: "var(--editor-current-stmt)" },
      ".cm-matchingBracket, .cm-nonmatchingBracket": {
        backgroundColor: "var(--editor-bracket)",
        outline: "1px solid var(--editor-bracket-outline)",
      },
      ".cm-selectionMatch": { backgroundColor: "var(--accent-soft)" },
      ".cm-searchMatch": { backgroundColor: "var(--search-match)", outline: "none" },
      ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: "var(--search-current)" },
      ".cm-foldPlaceholder": { backgroundColor: "transparent", border: "none", color: "var(--text-3)" },
      ".cm-tooltip": {
        backgroundColor: "var(--bg-elevated)",
        border: "1px solid var(--border-strong)",
        borderRadius: "6px",
        color: "var(--text)",
      },
      ".cm-tooltip-autocomplete > ul": { fontFamily: "var(--font-mono)", fontSize: "12px" },
      ".cm-tooltip-autocomplete > ul > li[aria-selected]": {
        backgroundColor: "var(--accent-tint)",
        color: "var(--text)",
      },
      ".cm-completionDetail": { color: "var(--text-3)", fontStyle: "normal", marginLeft: "12px" },
      ".cm-completionMatchedText": { textDecoration: "none", color: "var(--accent)", fontWeight: "700" },
      ".cm-panels": { backgroundColor: "var(--bg-chrome)", color: "var(--text)", borderColor: "var(--border)" },
      ".cm-panel.cm-search": { padding: "6px 8px", fontFamily: "var(--font-ui)", fontSize: "12px" },
      ".cm-panel.cm-search input, .cm-panel.cm-search button": { fontSize: "12px" },
      ".cm-textfield": {
        backgroundColor: "var(--bg)",
        border: "1px solid var(--border-strong)",
        borderRadius: "4px",
        color: "var(--text)",
      },
      ".cm-button": {
        backgroundImage: "none",
        backgroundColor: "var(--bg-elevated)",
        border: "1px solid var(--border-strong)",
        borderRadius: "4px",
        color: "var(--text)",
      },
      ".cm-lintRange-error": { backgroundImage: "none", textDecoration: "underline wavy var(--danger)" },
    },
    { dark },
  );
}

export function editorTheme(dark: boolean): Extension {
  return [chrome(dark), syntaxHighlighting(syntax)];
}
