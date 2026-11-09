// Formatting for the editor: SQL with sql-formatter, MongoDB shell queries with Prettier.
// Both run in the page; Prettier is loaded only the first time a MongoDB query is formatted.
import { formatDialect, mariadb, mysql, sqlite } from "sql-formatter";
import { splitStatements, type Dialect } from "./splitter";

/** Format one SQL statement or one MongoDB shell statement. Throws when it can't be parsed. */
export async function formatStatement(text: string, engine: string): Promise<string> {
  if (engine === "mongodb") return formatMongo(text);
  const dialect = engine === "sqlite" ? sqlite : engine === "mariadb" ? mariadb : mysql;
  return formatDialect(text, { dialect, keywordCase: "upper", tabWidth: 4 });
}

export interface FormatFailure {
  /** Line in the editor where the problem is. */
  line: number;
  message: string;
}

export interface FormatOutcome {
  text: string;
  /** Statements that were formatted (changed or already tidy). */
  formatted: number;
  /** Statements that couldn't be parsed: left exactly as they were. */
  failed: FormatFailure[];
}

/**
 * Format every statement in `src`, keeping what lies between them (separators, comments, blank
 * lines) as it is. A statement that can't be parsed is left as written; the others are still
 * formatted. `firstLine` is the editor line `src` starts on, for error messages.
 */
export async function formatText(src: string, dialect: Dialect, engine: string, firstLine = 1): Promise<FormatOutcome> {
  let text = "";
  let last = 0;
  let formatted = 0;
  const failed: FormatFailure[] = [];
  for (const st of splitStatements(src, dialect)) {
    text += src.slice(last, st.from);
    let out = st.text;
    try {
      out = await formatStatement(st.text, engine);
      formatted++;
    } catch (err) {
      // The line as it will read after formatting: statements above may have grown.
      const stmtLine = firstLine + countLines(text, text.length);
      failed.push({ line: stmtLine + errorLine(err) - 1, message: errorMessage(err) });
    }
    text += out;
    last = st.to;
  }
  return { text: text + src.slice(last), formatted, failed };
}

/** One line for the toolbar, or null when everything was formatted. */
export function describeFailures(outcome: FormatOutcome): string | null {
  const [first, ...more] = outcome.failed;
  if (!first) return null;
  let msg = `Couldn't format line ${first.line}: ${first.message}`;
  if (more.length) msg += ` (and ${more.length} more)`;
  if (outcome.formatted) msg += " · the other statements were formatted";
  return msg;
}

function countLines(src: string, upTo: number): number {
  let n = 0;
  for (let i = 0; i < upTo; i++) if (src.charCodeAt(i) === 10) n++;
  return n;
}

/** The 1-based line of a parse error within its statement (Prettier: err.loc; sql-formatter: "line N"). */
function errorLine(err: unknown): number {
  const loc = (err as { loc?: { start?: { line?: number } } })?.loc?.start?.line;
  if (typeof loc === "number" && loc > 0) return loc;
  const m = /\bline (\d+)/i.exec(err instanceof Error ? err.message : String(err));
  return m ? Number(m[1]) : 1;
}

/** The parser's message without Prettier's code frame or position suffix. */
function errorMessage(err: unknown): string {
  const msg = err instanceof Error ? err.message : String(err);
  return msg
    .split("\n")[0]
    .replace(/\s*\(\d+:\d+\)$/, "")
    .replace(/\s+at line \d+ column \d+.*$/i, "")
    .replace(/^Parse error:?\s*/i, "")
    // sql-formatter: "at token: «EOF»" / "at token: «)»"
    .replace(/^at token: «EOF»$/, "the statement ends too early")
    .replace(/^at token: «(.*)»$/, 'unexpected "$1"');
}

type Prettier = { format: typeof import("prettier/standalone").format; plugins: import("prettier").Plugin[] };
let prettier: Promise<Prettier> | null = null;

function loadPrettier(): Promise<Prettier> {
  prettier ??= Promise.all([
    import("prettier/standalone"),
    import("prettier/plugins/babel"),
    import("prettier/plugins/estree"),
  ]).then(([p, babel, estree]) => ({ format: p.format, plugins: [babel, estree as import("prettier").Plugin] }));
  return prettier;
}

/** mongosh helpers that aren't JavaScript ("show collections", "use shop"): left as written. */
const SHELL_COMMAND = /^\s*(show|use|it|exit|help)\b(?!\s*[.(=])/i;

async function formatMongo(text: string): Promise<string> {
  if (SHELL_COMMAND.test(text)) return text;
  const { format, plugins } = await loadPrettier();
  // A line break outside brackets ends a MongoDB statement unless the next line starts with "."
  // (see splitter.ts). Prettier's chains respect that; if a result ever would split, use a wider
  // line, and as a last resort leave the statement as it was.
  for (const printWidth of [80, 120, 1_000_000]) {
    let out = await format(text, {
      parser: "babel",
      plugins,
      printWidth,
      tabWidth: 2,
      singleQuote: true,
      semi: false,
      trailingComma: "none",
      arrowParens: "avoid",
    });
    out = out.replace(/^;/, "").replace(/\s+$/, "");
    if (splitStatements(out, "mongo").length === 1) return out;
  }
  return text;
}
