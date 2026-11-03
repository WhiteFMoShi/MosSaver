import assert from "node:assert/strict";
import test from "node:test";
import { parsePatch, previewPosition } from "../src/lib/diff.ts";

test("Git metadata is hidden and edits have separate old and new line numbers", () => {
  const lines = parsePatch([
    "diff --git a/a.txt b/a.txt", "--- a/a.txt", "+++ b/a.txt", "@@ -3,3 +3,4 @@",
    " unchanged", "-old", "+new", "+extra", " end", "",
  ].join("\n"));
  assert.equal(lines.length, 6);
  assert.deepEqual(lines.map(({ kind, oldNumber, newNumber, sign }) => [kind, oldNumber, newNumber, sign]), [
    ["hunk", null, null, ""], ["context", 3, 3, " "], ["deleted", 4, null, "-"],
    ["added", null, 4, "+"], ["added", null, 5, "+"], ["context", 5, 6, " "],
  ]);
  assert.equal(lines[0].text, "原文件 第 3–5 行 → 当前文件 第 3–6 行");
  assert.deepEqual(lines.slice(1).map((line) => line.text), ["unchanged", "old", "new", "extra", "end"]);
});

test("each hunk resets numbering and no-newline markers do not consume a line", () => {
  const lines = parsePatch("@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n@@ -100 +104,2 @@\n same\n+last");
  assert.deepEqual(lines.map(({ oldNumber, newNumber }) => [oldNumber, newNumber]), [
    [null, null], [1, null], [null, null], [null, 1], [null, null], [100, 104], [null, 105],
  ]);
  assert.deepEqual(lines.filter((line) => line.kind === "hunk").map((line) => line.text), [
    "原文件 第 1 行 → 当前文件 第 1 行",
    "原文件 第 100 行 → 当前文件 第 104–105 行",
  ]);
});

test("hunks describe added and removed files in plain language", () => {
  assert.equal(parsePatch("@@ -0,0 +1,2 @@\n+one\n+two")[0].text, "原文件 无对应行 → 当前文件 第 1–2 行");
  assert.equal(parsePatch("@@ -1,2 +0,0 @@\n-one\n-two")[0].text, "原文件 第 1–2 行 → 当前文件 无对应行");
});

test("new and deleted files number only the appropriate side, including empty lines", () => {
  assert.deepEqual(parsePatch("@@ -0,0 +1,2 @@\n+first\n+\n").slice(1).map(({ oldNumber, newNumber }) => [oldNumber, newNumber]), [[null, 1], [null, 2]]);
  assert.deepEqual(parsePatch("@@ -1,2 +0,0 @@\n-first\n-\n").slice(1).map(({ oldNumber, newNumber }) => [oldNumber, newNumber]), [[1, null], [2, null]]);
  assert.deepEqual(parsePatch(""), []);
});

test("patch text preserves HTML-like code, tabs and literal plus/minus content", () => {
  const lines = parsePatch("@@ -1 +1 @@\n---old\n+++<script>\t中文</script>\n");
  assert.equal(lines[1].kind, "deleted");
  assert.equal(lines[2].kind, "added");
  assert.equal(lines[2].sign, "+");
  assert.equal(lines[2].text, "++<script>\t中文</script>");
});

test("the popup opens to the pointer's right when there is room", () => {
  assert.deepEqual(previewPosition(100, 120, 1440, 900), { left: 118, top: 96, width: 620, height: 420 });
});

test("popup bounds stay inside every viewport edge and small windows", () => {
  for (const [width, height] of [[1440, 900], [800, 600], [320, 240]]) {
    for (const [x, y] of [[0, 0], [width, height], [width - 1, 1], [1, height - 1], [width / 2, height / 2]]) {
      const popup = previewPosition(x, y, width, height);
      assert.ok(popup.left >= 12 && popup.top >= 12);
      assert.ok(popup.left + popup.width <= width - 12);
      assert.ok(popup.top + popup.height <= height - 12);
      assert.ok(popup.width > 0 && popup.height > 0);
    }
  }
});
