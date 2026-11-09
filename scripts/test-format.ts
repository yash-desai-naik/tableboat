// Run with: node scripts/test-format.ts
// The editor's formatter: SQL via sql-formatter, MongoDB shell queries via Prettier.
import assert from "node:assert/strict";
import { registerHooks } from "node:module";

// The app's imports omit ".ts" (Vite resolves them); let Node do the same for this test.
registerHooks({
  resolve(spec, ctx, next) {
    try {
      return next(spec, ctx);
    } catch (err) {
      if (spec.startsWith(".")) return next(`${spec}.ts`, ctx);
      throw err;
    }
  },
});
const { describeFailures, formatText } = await import("../src/editor/format.ts");
const { splitStatements } = await import("../src/editor/splitter.ts");

// MongoDB: a long one-line pipeline becomes a readable chain and stays ONE statement.
const mongo = `db.getCollection('202603_debug_logs').aggregate([{ $match: { status: 'pending', createdAt: { $gte: ISODate('2026-03-01') } } }, { $group: { _id: '$service', n: { $sum: 1 } } }, { $sort: { n: -1 } }])`;
const m = await formatText(mongo, "mongo", "mongodb");
console.log(m.text + "\n");
assert.deepEqual(m.failed, []);
assert.equal(splitStatements(m.text, "mongo").length, 1, "formatted pipeline is still one statement");
assert.ok(m.text.includes("\n"), "long pipeline is broken over lines");
assert.ok(m.text.includes("ISODate('2026-03-01')"), "shell helpers and single quotes kept");

// Several statements, a shell command and a comment: each formatted, the rest left alone.
const many = `show collections\n// recent\ndb.users.find({active:true},{name:1,email:1}).sort({_id:-1}).limit(5);\ndb.orders.countDocuments({status:"paid"})`;
const mm = await formatText(many, "mongo", "mongodb");
console.log(mm.text + "\n");
assert.ok(mm.text.startsWith("show collections\n// recent\n"), "shell command and comment unchanged");
assert.ok(mm.text.includes("db.users\n  .find({ active: true }, { name: 1, email: 1 })\n  .sort({ _id: -1 })"), "chain broken per call, objects spaced");
assert.ok(mm.text.includes("db.orders.countDocuments({ status: 'paid' })"), "second statement formatted");
assert.equal(splitStatements(mm.text, "mongo").length, 3, "still three statements");
assert.ok(mm.text.includes(".limit(5);"), "the original ; separator is kept");

// One broken statement: the others are still formatted, the broken one is left exactly as is,
// and the message names the line where the parser gave up. The text starts on editor line 10,
// so the broken statement is on line 12.
const mixed = `db.a.find({x:1})\n\ndb.b.find({ a: ] })\ndb.c.find({y:2})`;
const mx = await formatText(mixed, "mongo", "mongodb", 10);
console.log(mx.text + "\n" + describeFailures(mx) + "\n");
assert.equal(mx.failed.length, 1);
assert.equal(mx.failed[0].line, 12, "error points at the broken line");
assert.ok(mx.text.startsWith("db.a.find({ x: 1 })"), "statement before it formatted");
assert.ok(mx.text.includes("db.b.find({ a: ] })"), "broken statement untouched");
assert.ok(mx.text.endsWith("db.c.find({ y: 2 })"), "statement after it formatted");
assert.match(describeFailures(mx) ?? "", /^Couldn't format line 12: .+ · the other statements were formatted$/);

// The reported line is where the broken text ends up AFTER formatting (the chain above grows
// from one line to four): the case from the user's screenshot.
const shot = `show collections\ndb.users.find({active:true},{name:1,email:1}).sort({_id:-1}).limit(5)\ndb.orders.countDocuments({ status: 'paid' })db.users.find({ a: `;
const sh = await formatText(shot, "mongo", "mongodb");
console.log(describeFailures(sh) + "\n");
assert.equal(sh.failed[0].line, 6, "line counted in the formatted text");
assert.equal(sh.text.split("\n")[5], "db.orders.countDocuments({ status: 'paid' })db.users.find({ a: ");

// SQL: keywords upper-cased, statements kept apart.
const sql = `select id,name from users where active=1 and created_at>'2026-01-01' order by id desc limit 10;\nupdate t set a=1 where id=2`;
const s = await formatText(sql, "mysql", "mysql");
console.log(s.text + "\n");
assert.ok(s.text.includes("SELECT\n    id,\n    name\nFROM\n    users"), "SELECT formatted");
assert.ok(s.text.includes(";\nUPDATE"), "separator kept between statements");
assert.equal(splitStatements(s.text, "mysql").length, 2);

// Broken SQL: reported with a line, text unchanged.
const bad = await formatText("select from where (", "mysql", "mysql", 4);
console.log(describeFailures(bad) + "\n");
assert.equal(bad.text, "select from where (");
assert.equal(bad.failed[0].line, 4);

console.log("format tests passed");
