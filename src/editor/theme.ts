import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import type { Extension } from "@codemirror/state";

/** VS Code "Dark+" token colours. */
const darkPlus = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.modifier, t.controlKeyword], color: "#569CD6" },
  { tag: [t.typeName, t.className], color: "#4EC9B0" },
  { tag: [t.standard(t.name), t.function(t.variableName)], color: "#DCDCAA" },
  { tag: [t.string, t.special(t.string)], color: "#CE9178" },
  { tag: [t.number], color: "#B5CEA8" },
  { tag: [t.bool, t.null, t.atom], color: "#569CD6" },
  { tag: [t.comment, t.lineComment, t.blockComment], color: "#6A9955" },
  { tag: [t.variableName, t.propertyName, t.special(t.name)], color: "#9CDCFE" },
  { tag: [t.name], color: "#D4D4D4" },
  { tag: [t.operator, t.punctuation, t.bracket, t.separator], color: "#D4D4D4" },
  { tag: t.invalid, color: "#F44747" },
]);

/** VS Code "Light+" token colours. */
const lightPlus = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.modifier, t.controlKeyword], color: "#0000FF" },
  { tag: [t.typeName, t.className], color: "#267F99" },
  { tag: [t.standard(t.name), t.function(t.variableName)], color: "#795E26" },
  { tag: [t.string, t.special(t.string)], color: "#A31515" },
  { tag: [t.number], color: "#098658" },
  { tag: [t.bool, t.null, t.atom], color: "#0000FF" },
  { tag: [t.comment, t.lineComment, t.blockComment], color: "#008000" },
  { tag: [t.variableName, t.propertyName, t.special(t.name)], color: "#001080" },
  { tag: [t.name], color: "#1F1F1F" },
  { tag: [t.operator, t.punctuation, t.bracket, t.separator], color: "#1F1F1F" },
  { tag: t.invalid, color: "#CD3131" },
]);

function chrome(dark: boolean): Extension {
  return EditorView.theme(
    {
      "&": {
        height: "100%",
        backgroundColor: "var(--bg)",
        color: dark ? "#D4D4D4" : "#1F1F1F",
        fontSize: "var(--editor-font-size)",
      },
      ".cm-scroller": {
        fontFamily: "var(--font-mono)",
        lineHeight: "1.55",
      },
      ".cm-content": { caretColor: dark ? "#AEAFAD" : "#000000", padding: "6px 0" },
      ".cm-cursor, .cm-dropCursor": { borderLeftColor: dark ? "#AEAFAD" : "#000000", borderLeftWidth: "2px" },
      "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, ::selection":
        { backgroundColor: dark ? "#264F78" : "#ADD6FF" },
      ".cm-gutters": {
        backgroundColor: "var(--bg)",
        color: dark ? "#6E7681" : "#9A9A9A",
        border: "none",
      },
      ".cm-activeLineGutter": { backgroundColor: "transparent", color: dark ? "#CCCCCC" : "#0B216F" },
      ".cm-activeLine": { backgroundColor: "transparent" },
      // The statement Ctrl+Enter will run.
      ".cm-current-stmt": { backgroundColor: dark ? "rgba(255,255,255,0.045)" : "rgba(0,0,0,0.035)" },
      ".cm-matchingBracket, .cm-nonmatchingBracket": {
        backgroundColor: dark ? "rgba(255,255,255,0.12)" : "rgba(0,0,0,0.1)",
        outline: dark ? "1px solid #888" : "1px solid #B9B9B9",
      },
      ".cm-selectionMatch": { backgroundColor: dark ? "rgba(173,214,255,0.15)" : "rgba(173,214,255,0.5)" },
      ".cm-searchMatch": { backgroundColor: dark ? "#623315" : "#F8C9AB", outline: "none" },
      ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: dark ? "#9E6A03" : "#F6B94D" },
      ".cm-foldPlaceholder": { backgroundColor: "transparent", border: "none", color: "var(--text-3)" },
      ".cm-tooltip": {
        backgroundColor: "var(--bg-elevated)",
        border: "1px solid var(--border-strong)",
        borderRadius: "6px",
        color: "var(--text)",
      },
      ".cm-tooltip-autocomplete > ul": { fontFamily: "var(--font-mono)", fontSize: "12px" },
      ".cm-tooltip-autocomplete > ul > li[aria-selected]": {
        backgroundColor: dark ? "#04395E" : "#D6EBFF",
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
      ".cm-lintRange-error": { backgroundImage: "none", textDecoration: "underline wavy #F44747" },
    },
    { dark },
  );
}

export function editorTheme(dark: boolean): Extension {
  return [chrome(dark), syntaxHighlighting(dark ? darkPlus : lightPlus)];
}
