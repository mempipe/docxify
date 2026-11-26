// Scores the browser's lines against Word's for compare-with-word.mjs.

/** The layout milestone in pipeline 00038/plan.md requires the same page count, and this share of lines broken the same way. */
export const LINES = 0.95;

/**
 * Compares the lines on each page, Word's in `theirs` and the browser's in
 * `ours`, as text with whitespace removed. A line is the same when its page
 * has it on both sides, each line counted once. The share is of whichever
 * side has more lines, so a line only the browser draws counts against it as
 * a line only Word draws does. `inOrder` is how many of the same lines also
 * come in the same order on their page. It is reported and not required:
 * pdftotext's order of a page's lines is not the order they are drawn in for
 * a header, a footnote or a table.
 */
export function score(theirs, ours) {
  let [same, inOrder, wordLines, ourLines] = [0, 0, 0, 0];
  const missing = [];
  const extra = [];
  theirs.forEach((lines, page) => {
    const left = [...(ours[page] ?? [])];
    wordLines += lines.length;
    for (const line of lines) {
      const at = left.indexOf(line);
      if (at >= 0) {
        same++;
        left.splice(at, 1);
      } else {
        missing.push(`p${page + 1}: ${line.slice(0, 60)}`);
      }
    }
    extra.push(...left.map((line) => `p${page + 1}: ${line.slice(0, 60)}`));
    inOrder += common(lines, ours[page] ?? []);
  });
  ours.forEach((lines, page) => {
    ourLines += lines.length;
    if (page >= theirs.length) extra.push(...lines.map((line) => `p${page + 1}: ${line.slice(0, 60)}`));
  });
  const total = Math.max(wordLines, ourLines);
  return { ok: theirs.length === ours.length && same >= LINES * total, same, total, inOrder, missing, extra };
}

/** The length of the longest run of lines two pages have in the same order. */
function common(a, b) {
  let row = new Array(b.length + 1).fill(0);
  for (const line of a) {
    const next = [0];
    for (let j = 0; j < b.length; j++) next.push(line === b[j] ? row[j] + 1 : Math.max(row[j + 1], next[j]));
    row = next;
  }
  return row[b.length];
}
