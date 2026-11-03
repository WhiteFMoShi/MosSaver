export interface DiffLine {
  text: string;
  kind: "header" | "hunk" | "added" | "deleted" | "context";
  sign: "+" | "-" | " " | "";
  oldNumber: number | null;
  newNumber: number | null;
}

function lineRange(start: number, count: number): string {
  if (count === 0) return "无对应行";
  const end = start + count - 1;
  return start === end ? `第 ${start} 行` : `第 ${start}–${end} 行`;
}

export function parsePatch(patch: string): DiffLine[] {
  let oldNumber = 0;
  let newNumber = 0;
  let inHunk = false;
  const lines: DiffLine[] = [];

  for (const text of patch.split("\n").filter((line, index, all) => index < all.length - 1 || line !== "")) {
    const hunk = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(?: ?(.*))?$/.exec(text);
    if (hunk) {
      oldNumber = Number(hunk[1]);
      newNumber = Number(hunk[3]);
      inHunk = true;
      const oldRange = lineRange(oldNumber, Number(hunk[2] ?? 1));
      const newRange = lineRange(newNumber, Number(hunk[4] ?? 1));
      const section = hunk[5]?.trim();
      lines.push({
        text: `原文件 ${oldRange} → 当前文件 ${newRange}${section ? ` · ${section}` : ""}`,
        kind: "hunk", sign: "", oldNumber: null, newNumber: null,
      });
      continue;
    }
    if (!inHunk) continue;
    if (text.startsWith("\\")) {
      lines.push({ text: "行末换行符不同", kind: "header", sign: "", oldNumber: null, newNumber: null });
    } else if (text.startsWith("+")) {
      lines.push({ text: text.slice(1), kind: "added", sign: "+", oldNumber: null, newNumber: newNumber++ });
    } else if (text.startsWith("-")) {
      lines.push({ text: text.slice(1), kind: "deleted", sign: "-", oldNumber: oldNumber++, newNumber: null });
    } else {
      lines.push({ text: text.slice(1), kind: "context", sign: " ", oldNumber: oldNumber++, newNumber: newNumber++ });
    }
  }
  return lines;
}

export function previewPosition(x: number, y: number, viewportWidth: number, viewportHeight: number) {
  const width = Math.min(620, viewportWidth - 24);
  const height = Math.min(420, viewportHeight - 24);
  return {
    left: Math.max(12, Math.min(x + 18, viewportWidth - width - 12)),
    top: Math.max(12, Math.min(y - 24, viewportHeight - height - 12)),
    width, height,
  };
}
