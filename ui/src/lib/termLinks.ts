// URLs on terminal rows, for the terminal's link provider. A URL may run
// over several rows: rows xterm wrapped itself (`wrapped`), and rows a TUI
// broke itself (Claude Code ends the row a few columns before the edge and
// indents the rest), which xterm sees as separate lines.

import type { ILink, ILinkProvider, Terminal } from '@xterm/xterm';

/** One terminal row: its text and whether xterm wrapped it from the previous row. */
export type Row = { text: string; wrapped: boolean };

/** A piece of a URL on one row: string indices, `end` exclusive. */
export type Seg = { row: number; start: number; end: number };

export type Found = { url: string; segs: Seg[] };

// The shape xterm's web-links addon uses: no spaces, quotes or brackets, and
// no trailing punctuation.
const START = /https?:\/\//gi;
const URL = /^https?:\/\/[^\s"'!*(){}|\\^<>`]*[^\s"':,.!?{}|\\^~[\]`()<>]/i;
const CHAR = /[^\s"'!*(){}|\\^<>`]/;

/** How close to the right edge a row must end to count as broken by the TUI
 * (Claude Code leaves about 8 columns). */
const EDGE = 12;

/** End of the run of URL characters in `text` from `from`. */
function run(text: string, from: number): number {
  let i = from;
  while (i < text.length && CHAR.test(text[i])) i++;
  return i;
}

/** URLs found in `rows`, a terminal `cols` wide, with the rows they cover. */
export function findUrls(rows: Row[], cols: number): Found[] {
  const found: Found[] = [];
  let resume = 0; // where to look on the current row (after a URL that ended on it)
  for (let r = 0; r < rows.length; r++) {
    const text = rows[r].text;
    START.lastIndex = resume;
    resume = 0;
    for (let m; (m = START.exec(text)); ) {
      // The URL's text, row by row, while it fills its row to the end.
      const segs: Seg[] = [{ row: r, start: m.index, end: run(text, m.index) }];
      for (let row = r; ; row++) {
        const cur = rows[row].text;
        const next = rows[row + 1];
        if (!next || segs.at(-1)!.end < cur.trimEnd().length) break;
        const indent = next.wrapped ? 0 : next.text.length - next.text.trimStart().length;
        if (!next.wrapped && cur.trimEnd().length < cols - EDGE) break;
        const end = run(next.text, indent);
        if (end === indent) break;
        segs.push({ row: row + 1, start: indent, end });
      }
      // Cut to the URL proper (no trailing punctuation), segments to match.
      const joined = segs.map((s) => rows[s.row].text.slice(s.start, s.end)).join('');
      const url = URL.exec(joined)?.[0];
      if (!url) continue;
      let left = url.length;
      const kept: Seg[] = [];
      for (const s of segs) {
        if (left <= 0) break;
        const len = Math.min(s.end - s.start, left);
        kept.push({ row: s.row, start: s.start, end: s.start + len });
        left -= len;
      }
      found.push({ url, segs: kept });
      const last = kept.at(-1)!;
      if (last.row !== r) {
        // Go on from the row where this URL ended, after it.
        r = last.row - 1;
        resume = last.end;
        break;
      }
      START.lastIndex = last.end;
    }
  }
  return found;
}

const ROWS = 40; // rows looked at above and below the hovered one

/** Buffer row `y` (0-based) as text; `cols[i]` is the cell of `text[i]`
 * (a wide character takes two cells). */
function rowAt(term: Terminal, y: number) {
  const line = term.buffer.active.getLine(y);
  if (!line) return undefined;
  let text = '';
  const cols: number[] = [];
  for (let x = 0; x < line.length; x++) {
    const cell = line.getCell(x);
    if (!cell || cell.getWidth() === 0) continue;
    const ch = cell.getChars() || ' ';
    for (let i = 0; i < ch.length; i++) cols.push(x);
    text += ch;
  }
  return { text, wrapped: line.isWrapped, cols };
}

/** The terminal's link provider for plain URLs: each row's piece of a URL
 * is a link to the whole URL. */
export function urlLinks(term: Terminal, open: (e: MouseEvent, uri: string) => void): ILinkProvider {
  return {
    provideLinks(y, done) {
      const at = y - 1; // provideLinks is 1-based
      const from = Math.max(0, at - ROWS);
      const rows = [];
      for (let i = from; i <= at + ROWS; i++) {
        const r = rowAt(term, i);
        if (!r) break;
        rows.push(r);
      }
      const out: ILink[] = [];
      for (const f of findUrls(rows, term.cols)) {
        for (const s of f.segs) {
          if (s.row + from !== at || s.end <= s.start) continue;
          const c = rows[s.row].cols;
          out.push({
            text: f.url,
            range: { start: { x: c[s.start] + 1, y }, end: { x: c[s.end - 1] + 1, y } },
            activate: open,
          });
        }
      }
      done(out.length ? out : undefined);
    },
  };
}
