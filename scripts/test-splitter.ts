// Run with: node scripts/test-splitter.ts
import assert from "node:assert/strict";
import { splitStatements, statementAt } from "../src/editor/splitter.ts";

const texts = (sql: string, d: "mysql" | "sqlite" = "mysql") => splitStatements(sql, d).map((s) => s.text);
let passed = 0;
const test = (name: string, fn: () => void) => {
  fn();
  passed++;
  console.log("ok -", name);
};

test("basic", () => {
  assert.deepEqual(texts("SELECT 1; SELECT 2;\nSELECT 3"), ["SELECT 1", "SELECT 2", "SELECT 3"]);
});

test("semicolons in strings, identifiers and comments", () => {
  assert.deepEqual(
    texts("SELECT 'a;b', \"c;d\", `e;f` -- x;y\n; # z;w\nSELECT /* ; */ 2;"),
    ["SELECT 'a;b', \"c;d\", `e;f` -- x;y", "SELECT /* ; */ 2"],
  );
});

test("escaped quotes", () => {
  assert.deepEqual(texts("SELECT 'it''s; ok', 'a\\';b'; SELECT 2"), ["SELECT 'it''s; ok', 'a\\';b'", "SELECT 2"]);
  // In SQLite a backslash is not an escape.
  assert.deepEqual(texts("SELECT 'a\\'; SELECT 2", "sqlite"), ["SELECT 'a\\'", "SELECT 2"]);
});

test("MySQL -- needs a space; # is a comment", () => {
  assert.deepEqual(texts("SELECT 5--1; SELECT 2"), ["SELECT 5--1", "SELECT 2"]);
  assert.deepEqual(texts("SELECT 1 #; comment\n; SELECT 2"), ["SELECT 1 #; comment", "SELECT 2"]);
});

test("comment-only parts are skipped", () => {
  assert.deepEqual(texts("-- just a note\n;\n/* x */;\nSELECT 1;"), ["SELECT 1"]);
});

test("DELIMITER blocks", () => {
  const sql = [
    "DELIMITER $$",
    "CREATE PROCEDURE p()",
    "BEGIN",
    "  SELECT 1; -- inside",
    "  SELECT 2;",
    "END$$",
    "DELIMITER ;",
    "CALL p();",
  ].join("\n");
  const t = texts(sql);
  assert.equal(t.length, 2);
  assert.ok(t[0].startsWith("CREATE PROCEDURE p()") && t[0].endsWith("END"));
  assert.equal(t[1], "CALL p()");
});

test("compound body without DELIMITER", () => {
  const sql = "CREATE TRIGGER t AFTER INSERT ON x FOR EACH ROW BEGIN\n IF NEW.a > 1 THEN SET @y = 1; END IF;\n UPDATE z SET b = CASE WHEN 1 THEN 2 END;\nEND;\nSELECT 3;";
  const t = texts(sql);
  assert.equal(t.length, 2, JSON.stringify(t));
  assert.ok(t[0].endsWith("END"));
  assert.equal(t[1], "SELECT 3");
});

test("BEGIN as a transaction is not a block", () => {
  assert.deepEqual(texts("BEGIN; UPDATE a SET b = 1; COMMIT;"), ["BEGIN", "UPDATE a SET b = 1", "COMMIT"]);
});

test("SQLite trigger and bracket identifiers", () => {
  const sql = "CREATE TRIGGER tr AFTER DELETE ON [my;table] BEGIN DELETE FROM b WHERE id = old.id; END; SELECT 1;";
  assert.deepEqual(texts(sql, "sqlite").length, 2);
});

test("statement under the cursor", () => {
  const sql = "SELECT 1;\n\nSELECT 2;   \nSELECT 3";
  const st = splitStatements(sql, "mysql");
  assert.equal(statementAt(st, 0)?.text, "SELECT 1");
  assert.equal(statementAt(st, sql.indexOf("2;") + 2)?.text, "SELECT 2", "right after the semicolon");
  assert.equal(statementAt(st, sql.indexOf("2;") + 5)?.text, "SELECT 2", "trailing spaces on the same line");
  assert.equal(statementAt(st, sql.length)?.text, "SELECT 3");
});

test("200 statements", () => {
  const sql = Array.from({ length: 200 }, (_, i) => `INSERT INTO t VALUES (${i}, 'a;b');`).join("\n");
  assert.equal(texts(sql).length, 200);
});

const mongo = (s: string) => splitStatements(s, "mongo").map((x) => x.text);

test("MongoDB: one statement per line, chains and multi-line objects", () => {
  const src = [
    "db.users.find({ age: { $gt: 30 } })",
    "  .sort({ name: 1 })",
    "  .limit(5)",
    "// a comment; with a semicolon",
    "db.orders.aggregate([",
    "  { $match: { status: 'paid; or not' } },",
    "  { $group: { _id: '$user', n: { $sum: 1 } } }",
    "])",
    "show collections; db.x.countDocuments({})",
  ].join("\n");
  const t = mongo(src);
  assert.equal(t.length, 4, JSON.stringify(t));
  assert.ok(t[0].startsWith("db.users.find") && t[0].endsWith(".limit(5)"));
  assert.ok(t[1].startsWith("db.orders.aggregate([") && t[1].endsWith("])"));
  assert.equal(t[2], "show collections");
  assert.equal(t[3], "db.x.countDocuments({})");
});

test("MongoDB: statement under the cursor", () => {
  const src = "db.a.find()\n\ndb.b.find({\n  x: 1\n})";
  const st = splitStatements(src, "mongo");
  assert.equal(statementAt(st, src.indexOf("x: 1"))?.text, "db.b.find({\n  x: 1\n})");
});

console.log(`\n${passed} passed`);
