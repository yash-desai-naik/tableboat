import { useEffect, useRef } from "react";
import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { MySQL, MariaSQL, SQLite, sql, type SQLNamespace } from "@codemirror/lang-sql";
import { javascript } from "@codemirror/lang-javascript";
import { bracketMatching, foldGutter, foldKeymap, indentOnInput } from "@codemirror/language";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState, StateField, type Extension } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  drawSelection,
  dropCursor,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  rectangularSelection,
  crosshairCursor,
} from "@codemirror/view";
// Only the dialects Tableboat uses, so the other formatters aren't bundled.
import { formatDialect, mariadb, mysql, sqlite } from "sql-formatter";
import { queryApi } from "../lib/query";
import { tabClosedHooks } from "../lib/workspace";
import { schemaCacheClearers } from "./schemaCache";
import { editorChanged, editors, hiddenEditorText, type EditorHandle } from "./registry";
import { splitStatements, statementAt, type Dialect, type Statement } from "./splitter";
import { editorTheme } from "./theme";

/** Editor states by tab, so text, cursor and undo history survive switching tabs. */
const savedStates = new Map<string, EditorState>();
tabClosedHooks.add((id) => savedStates.delete(id));
hiddenEditorText.get = (id) => savedStates.get(id)?.doc.toString();


/** Autocomplete schema per session+database, loaded once. */
const schemaCache = new Map<string, Promise<SQLNamespace>>();

function loadSchema(sessionId: string, database: string): Promise<SQLNamespace> {
  const key = `${sessionId}\u0000${database}`;
  let p = schemaCache.get(key);
  if (!p) {
    p = queryApi
      .columns(sessionId, database)
      .then((tables) => {
        const ns: Record<string, { label: string; type: string; detail: string }[]> = {};
        for (const [table, cols] of Object.entries(tables)) {
          ns[table] = cols.map((c) => ({ label: c.name, type: "property", detail: c.typeName }));
        }
        return ns as SQLNamespace;
      })
      .catch(() => ({}) as SQLNamespace);
    schemaCache.set(key, p);
  }
  return p;
}

schemaCacheClearers.add(() => schemaCache.clear());

/** Lines of the statement under the cursor get a faint background. */
const MAX_HIGHLIGHT_DOC = 500_000;

const fieldCache = new Map<Dialect, ReturnType<typeof buildStatementField>>();
function currentStatementField(dialect: Dialect) {
  let f = fieldCache.get(dialect);
  if (!f) {
    f = buildStatementField(dialect);
    fieldCache.set(dialect, f);
  }
  return f;
}

/** Shared reconfiguration handles (a restored editor state must use the same ones). */
const themeC = new Compartment();
const wrapC = new Compartment();
const langC = new Compartment();

function buildStatementField(dialect: Dialect) {
  const statements = StateField.define<Statement[]>({
    create: (s) => (s.doc.length > MAX_HIGHLIGHT_DOC ? [] : splitStatements(s.doc.toString(), dialect)),
    update: (value, tr) =>
      tr.docChanged
        ? tr.state.doc.length > MAX_HIGHLIGHT_DOC
          ? []
          : splitStatements(tr.state.doc.toString(), dialect)
        : value,
  });
  const decorations = StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update: (_value, tr) => {
      const state = tr.state;
      const sel = state.selection.main;
      if (!sel.empty || state.selection.ranges.length > 1) return Decoration.none;
      const stmt = statementAt(state.field(statements), sel.head);
      if (!stmt) return Decoration.none;
      const deco = Decoration.line({ class: "cm-current-stmt" });
      const first = state.doc.lineAt(stmt.from).number;
      const last = state.doc.lineAt(stmt.to).number;
      const ranges = [];
      for (let n = first; n <= last; n++) ranges.push(deco.range(state.doc.line(n).from));
      return Decoration.set(ranges);
    },
    provide: (f) => EditorView.decorations.from(f),
  });
  return { statements, extension: [statements, decorations] as Extension };
}

interface Props {
  tabId: string;
  initialText: string;
  engine: string;
  sessionId: string;
  database: string | null;
  dark: boolean;
  wordWrap: boolean;
}

export function SqlEditor({ tabId, initialText, engine, sessionId, database, dark, wordWrap }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const dialect: Dialect = engine === "mongodb" ? "mongo" : engine === "sqlite" ? "sqlite" : "mysql";
  const isMongo = engine === "mongodb";

  // Create the editor (or restore this tab's previous state).
  useEffect(() => {
    if (!host.current) return;
    const c = { theme: themeC, wrap: wrapC, lang: langC };
    const current = currentStatementField(dialect);
    const sqlDialect = engine === "sqlite" ? SQLite : engine === "mariadb" ? MariaSQL : MySQL;

    const extensions: Extension[] = [
      lineNumbers(),
      foldGutter({ openText: "⌄", closedText: "›" }),
      highlightActiveLineGutter(),
      highlightSpecialChars(),
      history(),
      // Solid cursor: a blinking one repaints twice a second even when idle.
      drawSelection({ cursorBlinkRate: 0 }),
      dropCursor(),
      EditorState.allowMultipleSelections.of(true),
      indentOnInput(),
      bracketMatching(),
      closeBrackets(),
      autocompletion({ activateOnTyping: true, maxRenderedOptions: 80 }),
      rectangularSelection(),
      crosshairCursor(),
      highlightActiveLine(),
      highlightSelectionMatches(),
      keymap.of([
        ...closeBracketsKeymap,
        ...defaultKeymap,
        ...searchKeymap,
        ...historyKeymap,
        ...foldKeymap,
        ...completionKeymap,
        indentWithTab,
      ]),
      current.extension,
      c.theme.of(editorTheme(dark)),
      c.wrap.of(wordWrap ? EditorView.lineWrapping : []),
      c.lang.of(isMongo ? javascript() : sql({ dialect: sqlDialect, upperCaseKeywords: false })),
      EditorView.contentAttributes.of({ spellcheck: "false", autocorrect: "off", autocapitalize: "off" }),
      // Lets the open tabs (and their SQL) be saved for next time.
      EditorView.updateListener.of((u) => {
        if (u.docChanged) editorChanged.notify();
      }),
    ];

    const state = savedStates.get(tabId) ?? EditorState.create({ doc: initialText, extensions });
    const view = new EditorView({ state, parent: host.current });
    // A restored state keeps its own extensions; re-apply the current theme and wrap.
    view.dispatch({
      effects: [
        c.theme.reconfigure(editorTheme(dark)),
        c.wrap.reconfigure(wordWrap ? EditorView.lineWrapping : []),
      ],
    });
    viewRef.current = view;

    const stmtField = current.statements;
    const handle: EditorHandle = {
      currentStatements() {
        const s = view.state;
        const parts = s.selection.ranges.filter((r) => !r.empty).map((r) => s.sliceDoc(r.from, r.to));
        if (parts.length) return parts.flatMap((p) => splitStatements(p, dialect).map((x) => x.text));
        const statements = s.field(stmtField, false) ?? splitStatements(s.doc.toString(), dialect);
        const list = statements.length ? statements : splitStatements(s.doc.toString(), dialect);
        const stmt = statementAt(list, s.selection.main.head);
        return stmt ? [stmt.text] : [];
      },
      allStatements() {
        return splitStatements(view.state.doc.toString(), dialect).map((s) => s.text);
      },
      format() {
        const s = view.state;
        const fmtDialect = engine === "sqlite" ? sqlite : engine === "mariadb" ? mariadb : mysql;
        if (isMongo) return "Formatting is for SQL; MongoDB queries are left as written.";
        const sel = s.selection.main;
        let from = sel.from;
        let to = sel.to;
        if (sel.empty) {
          const stmt = statementAt(s.field(stmtField, false) ?? splitStatements(s.doc.toString(), dialect), sel.head);
          if (!stmt) return null;
          from = stmt.from;
          to = stmt.to;
        }
        try {
          const formatted = formatDialect(s.sliceDoc(from, to), { dialect: fmtDialect, keywordCase: "upper", tabWidth: 4 });
          view.dispatch({ changes: { from, to, insert: formatted }, selection: { anchor: from + formatted.length } });
          return null;
        } catch (err) {
          return err instanceof Error ? err.message : String(err);
        }
      },
      focus() {
        view.focus();
      },
      text() {
        return view.state.doc.toString();
      },
      insert(text: string) {
        const sel = view.state.selection.main;
        // Inserted SQL starts on its own line, separated from what's already there.
        const before = view.state.sliceDoc(Math.max(0, sel.from - 2), sel.from);
        const prefix = sel.from === 0 || before.endsWith("\n\n") ? "" : before.endsWith("\n") ? "\n" : "\n\n";
        const insert = prefix + text;
        view.dispatch({ changes: { from: sel.from, to: sel.to, insert }, selection: { anchor: sel.from + insert.length } });
        view.focus();
      },
    };
    editors.set(tabId, handle);
    view.focus();

    return () => {
      savedStates.set(tabId, view.state);
      editors.delete(tabId);
      view.destroy();
      viewRef.current = null;
    };
    // Recreated only per tab; theme, wrap and schema are reconfigured below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tabId]);

  useEffect(() => {
    viewRef.current?.dispatch({ effects: themeC.reconfigure(editorTheme(dark)) });
  }, [dark]);

  useEffect(() => {
    viewRef.current?.dispatch({
      effects: wrapC.reconfigure(wordWrap ? EditorView.lineWrapping : []),
    });
  }, [wordWrap]);

  // Table and column names for autocomplete, once per database.
  useEffect(() => {
    if (!database || engine === "mongodb") return;
    let stale = false;
    const sqlDialect = engine === "sqlite" ? SQLite : engine === "mariadb" ? MariaSQL : MySQL;
    void loadSchema(sessionId, database).then((schema) => {
      if (stale || !viewRef.current) return;
      viewRef.current.dispatch({
        effects: langC.reconfigure(sql({ dialect: sqlDialect, schema, upperCaseKeywords: false })),
      });
    });
    return () => {
      stale = true;
    };
  }, [sessionId, database, engine, tabId]);

  return <div ref={host} className="sql-editor" />;
}
