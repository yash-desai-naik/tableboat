/**
 * Splits SQL text into statements, for "run current" and "run all".
 *
 * Handles: '…' "…" `…` [ … ] quoting (with '' and \' escapes), -- and # line comments,
 * /* … *\/ block comments, MySQL `DELIMITER` lines, and compound bodies
 * (CREATE PROCEDURE / FUNCTION / TRIGGER / EVENT … BEGIN … END) whose inner `;` must
 * not end the statement.
 */

export type Dialect = "mysql" | "sqlite" | "mongo";

export interface Statement {
  /** Offset of the first character that isn't whitespace. */
  from: number;
  /** Offset just after the last character before the delimiter. */
  to: number;
  /** Offset just after the delimiter (or `to` for the last statement without one). */
  end: number;
  text: string;
}

const COMPOUND = /^\s*(?:CREATE\b[\s\S]*?\b(?:PROCEDURE|FUNCTION|TRIGGER|EVENT)\b|ALTER\s+EVENT\b)/i;

function isWordChar(c: string) {
  return /[A-Za-z0-9_$]/.test(c);
}

export function splitStatements(src: string, dialect: Dialect): Statement[] {
  if (dialect === "mongo") return splitMongo(src);
  const out: Statement[] = [];
  const n = src.length;
  let delimiter = ";";
  let depth = 0; // BEGIN/CASE nesting, only counted inside compound statements
  let compound: boolean | null = null; // decided once the statement's first words are known
  let hasCode = false; // the statement contains something other than comments/whitespace
  let firstCode = -1;
  let i = 0;
  let atLineStart = true;

  const push = (to: number, end: number) => {
    if (hasCode && firstCode >= 0) {
      let t = to;
      while (t > firstCode && /\s/.test(src[t - 1])) t--;
      out.push({ from: firstCode, to: t, end, text: src.slice(firstCode, t) });
    }
    depth = 0;
    compound = null;
    hasCode = false;
    firstCode = -1;
  };

  while (i < n) {
    const c = src[i];

    // DELIMITER only means something at the start of a line, outside a statement body.
    if (dialect === "mysql" && atLineStart) {
      const m = /^[ \t]*DELIMITER[ \t]+(\S+)[ \t]*(?:\r?\n|$)/i.exec(src.slice(i, i + 200));
      if (m && !hasCode) {
        delimiter = m[1];
        i += m[0].length;
        atLineStart = true;
        continue;
      }
    }

    if (c === "\n") {
      atLineStart = true;
      i++;
      continue;
    }
    if (c === " " || c === "\t" || c === "\r") {
      i++;
      continue;
    }
    atLineStart = false;

    // Comments.
    if (c === "-" && src[i + 1] === "-" && (dialect === "sqlite" || i + 2 >= n || /\s/.test(src[i + 2]))) {
      while (i < n && src[i] !== "\n") i++;
      continue;
    }
    if (c === "#" && dialect === "mysql") {
      while (i < n && src[i] !== "\n") i++;
      continue;
    }
    if (c === "/" && src[i + 1] === "*") {
      const close = src.indexOf("*/", i + 2);
      i = close < 0 ? n : close + 2;
      continue;
    }

    // Delimiter.
    if (src.startsWith(delimiter, i) && (delimiter !== ";" || depth <= 0)) {
      push(i, i + delimiter.length);
      i += delimiter.length;
      continue;
    }

    if (!hasCode) {
      hasCode = true;
      firstCode = i;
    }

    // Quoted strings and identifiers.
    if (c === "'" || c === '"' || c === "`") {
      i++;
      while (i < n) {
        if (src[i] === "\\" && c !== "`" && dialect === "mysql") {
          i += 2;
          continue;
        }
        if (src[i] === c) {
          if (src[i + 1] === c) {
            i += 2;
            continue;
          }
          i++;
          break;
        }
        i++;
      }
      continue;
    }
    if (c === "[" && dialect === "sqlite") {
      const close = src.indexOf("]", i + 1);
      i = close < 0 ? n : close + 1;
      continue;
    }

    // Words: track BEGIN/CASE … END nesting inside compound statements.
    if (isWordChar(c)) {
      let j = i;
      // Stop at a custom delimiter such as "$$" in "END$$" ($ is otherwise a word character).
      while (j < n && isWordChar(src[j]) && !(delimiter !== ";" && src.startsWith(delimiter, j))) j++;
      const word = src.slice(i, j).toUpperCase();
      if (compound === null && /^(CREATE|ALTER|BEGIN|DECLARE|SELECT|INSERT|UPDATE|DELETE|WITH|SET|SHOW|USE|DROP|CALL)$/.test(word)) {
        compound = COMPOUND.test(src.slice(firstCode, Math.min(n, firstCode + 400)));
      }
      if (compound) {
        if (word === "BEGIN" || word === "CASE") depth++;
        else if (word === "END") {
          // END IF / END LOOP / END WHILE / END REPEAT close blocks we don't count.
          const rest = /^\s+([A-Za-z]+)/.exec(src.slice(j, j + 20));
          const next = rest?.[1].toUpperCase();
          if (next !== "IF" && next !== "LOOP" && next !== "WHILE" && next !== "REPEAT") depth--;
        }
      }
      i = j;
      continue;
    }
    i++;
  }
  push(n, n);
  return out;
}

/**
 * MongoDB shell statements end at ";" or at a line break outside any brackets, unless the next
 * line continues the chain with "." (e.g. a .sort() on its own line). Comments are // and /* *\/.
 */
function splitMongo(src: string): Statement[] {
  const out: Statement[] = [];
  const n = src.length;
  let depth = 0;
  let first = -1;
  let i = 0;
  const push = (to: number, end: number) => {
    if (first >= 0) {
      let t = to;
      while (t > first && /\s/.test(src[t - 1])) t--;
      out.push({ from: first, to: t, end, text: src.slice(first, t) });
    }
    first = -1;
    depth = 0;
  };
  while (i < n) {
    const c = src[i];
    if (c === "/" && src[i + 1] === "/") {
      while (i < n && src[i] !== "\n") i++;
      continue;
    }
    if (c === "/" && src[i + 1] === "*") {
      const close = src.indexOf("*/", i + 2);
      i = close < 0 ? n : close + 2;
      continue;
    }
    if (c === "\n" && depth <= 0 && first >= 0) {
      let j = i + 1;
      while (j < n && /\s/.test(src[j])) j++;
      if (src[j] !== ".") {
        push(i, i);
        i++;
        continue;
      }
    }
    if (/\s/.test(c)) {
      i++;
      continue;
    }
    if (c === ";" && depth <= 0) {
      push(i, i + 1);
      i++;
      continue;
    }
    if (first < 0) first = i;
    if (c === "'" || c === '"' || c === "`") {
      i++;
      while (i < n && src[i] !== c) i += src[i] === "\\" ? 2 : 1;
      i++;
      continue;
    }
    if (c === "(" || c === "[" || c === "{") depth++;
    else if (c === ")" || c === "]" || c === "}") depth--;
    i++;
  }
  push(n, n);
  return out;
}

/** The statement the cursor is in, or the closest one before it (TablePlus behaviour). */
export function statementAt(statements: Statement[], pos: number): Statement | null {
  if (statements.length === 0) return null;
  let best: Statement | null = null;
  for (const s of statements) {
    if (s.from <= pos) best = s;
    if (pos >= s.from && pos <= s.end) return s;
  }
  return best ?? statements[0];
}
