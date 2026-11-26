import assert from "node:assert/strict";
import { test } from "node:test";

import { score } from "./word-score.mjs";

test("the same lines on the same pages pass", () => {
  const pages = [["Introduction", "Results"], ["Conclusion"]];
  const { ok, same, total, inOrder } = score(pages, pages);
  assert.deepEqual({ ok, same, total, inOrder }, { ok: true, same: 3, total: 3, inOrder: 3 });
});

test("a line only the browser draws counts against the layout", () => {
  const result = score([["Introduction", "Results"]], [["Results", "Extra paragraph", "Introduction"]]);
  assert.equal(result.ok, false);
  assert.deepEqual([result.same, result.total, result.inOrder], [2, 3, 1]);
  assert.deepEqual(result.extra, ["p1: Extra paragraph"]);
});

test("a duplicated paragraph does not pass", () => {
  const word = [Array.from({ length: 10 }, (_, i) => `Line ${i}`)];
  const result = score(word, [[...word[0], ...word[0]]]);
  assert.equal(result.ok, false);
  assert.equal(result.extra.length, 10);
});

test("a line Word has and the browser does not is named", () => {
  const result = score([["One", "Two"]], [["One"]]);
  assert.equal(result.ok, false);
  assert.deepEqual(result.missing, ["p1: Two"]);
});

test("another page count does not pass", () => {
  assert.equal(score([["One"], ["Two"]], [["One", "Two"]]).ok, false);
});
