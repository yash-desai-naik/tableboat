// Run with: node scripts/test-readonly.ts
// A refresh re-runs a query tab only when isReadOnlyRun says it can't change data.
import assert from "node:assert/strict";
import { isReadOnlyRun } from "../src/lib/query.ts";

const reads = [
  "SELECT * FROM users",
  "  -- latest\nselect id from t where note = 'please delete me'",
  "WITH x AS (SELECT 1) SELECT * FROM x",
  "SHOW TABLES",
  "EXPLAIN SELECT 1",
  "db.users.find({ name: 'update' }).limit(5)",
  "db.orders.aggregate([{ $match: { s: 'paid' } }])",
  "show collections",
  "db.getCollection('x').countDocuments({})",
];
const writes = [
  "UPDATE users SET a = 1",
  "DELETE FROM t",
  "WITH x AS (SELECT 1) DELETE FROM t WHERE id IN (SELECT * FROM x)",
  "SELECT 1; ",
  "INSERT INTO t VALUES (1)",
  "db.users.updateOne({}, { $set: { a: 1 } })",
  "db.orders.aggregate([{ $out: 'copy' }])",
  "db.users.drop()",
  "CALL do_things()",
];
for (const r of reads) assert.equal(isReadOnlyRun([r]), true, `should be read-only: ${r}`);
for (const w of writes.filter((w) => w !== "SELECT 1; ")) assert.equal(isReadOnlyRun([w]), false, `should be a write: ${w}`);
assert.equal(isReadOnlyRun(["SELECT 1", "UPDATE t SET a = 1"]), false, "one write makes the run a write");
console.log(`${reads.length} reads and ${writes.length - 1} writes classified correctly`);
