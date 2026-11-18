// Unified diff parsing for the Changes tab, and a small line diff for
// "changed since you approved".

export interface Line {
  kind: 'a' | 'd' | 'c' | 'h';
  old?: number;
  new?: number;
  text: string;
  /** On a hunk header row (or the trailing row after the last hunk): the hidden lines it can reveal. */
  gap?: { id: number; hidden: number | null; up: boolean; down: boolean };
}
export interface FileDiff {
  path: string;
  status: 'added' | 'deleted' | 'modified' | 'renamed' | 'binary' | 'too large';
  adds: number;
  dels: number;
  lines: Line[];
  /** Why there is no diff: the daemon's `Not shown: …` line for a file over its size limit. */
  note?: string;
}

/** Lines revealed per click on an expand button. */
export const STEP = 20;

/** First line and length of each side of a hunk header; `null` when `text` is not one.
 *  An empty side (length 0) starts at the line after the one git names. */
export function hunkStart(text: string): { old: number; new: number; oldLen: number; newLen: number } | null {
  const m = text.match(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/);
  if (!m) return null;
  const oldLen = m[2] === undefined ? 1 : +m[2];
  const newLen = m[4] === undefined ? 1 : +m[4];
  return { old: +m[1] + (oldLen === 0 ? 1 : 0), new: +m[3] + (newLen === 0 ? 1 : 0), oldLen, newLen };
}

export function parseUnified(diff: string): FileDiff[] {
  const files: FileDiff[] = [];
  let f: FileDiff | null = null;
  let o = 0, n = 0;
  for (const raw of diff.split('\n')) {
    if (raw.startsWith('diff --git ')) {
      const m = raw.match(/^diff --git a\/(.*) b\/(.*)$/);
      f = { path: m ? m[2] : raw.slice(11), status: 'modified', adds: 0, dels: 0, lines: [] };
      files.push(f);
      continue;
    }
    if (!f) continue;
    if (raw.startsWith('new file mode')) f.status = 'added';
    else if (raw.startsWith('deleted file mode')) f.status = 'deleted';
    else if (raw.startsWith('rename to ')) { f.status = 'renamed'; f.path = raw.slice(10); }
    else if (raw.startsWith('Binary files')) f.status = 'binary';
    else if (raw.startsWith('Not shown: ')) { f.status = 'too large'; f.note = raw; }
    else if (raw.startsWith('@@')) {
      const m = raw.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      o = m ? +m[1] : 0;
      n = m ? +m[2] : 0;
      f.lines.push({ kind: 'h', text: raw });
    } else if (raw.startsWith('+++') || raw.startsWith('---') || raw.startsWith('index ')) continue;
    else if (raw.startsWith('+')) { f.adds++; f.lines.push({ kind: 'a', new: n++, text: raw.slice(1) }); }
    else if (raw.startsWith('-')) { f.dels++; f.lines.push({ kind: 'd', old: o++, text: raw.slice(1) }); }
    else if (raw.startsWith(' ')) f.lines.push({ kind: 'c', old: o++, new: n++, text: raw.slice(1) });
  }
  return files;
}

/** The files whose path contains `q`, ignoring case and surrounding spaces (R16). */
export function filterFiles<T extends { path: string }>(files: T[], q: string): T[] {
  const s = q.trim().toLowerCase();
  return s ? files.filter((f) => f.path.toLowerCase().includes(s)) : files;
}

/** Line diff (LCS) of two texts; context lines kept so the result reads in full. */
export function lineDiff(before: string, after: string): Line[] {
  const a = before.split('\n'), b = after.split('\n');
  if (a.length * b.length > 4_000_000) {
    return [...a.map((t, i) => ({ kind: 'd' as const, old: i + 1, text: t })), ...b.map((t, i) => ({ kind: 'a' as const, new: i + 1, text: t }))];
  }
  const L: number[][] = Array.from({ length: a.length + 1 }, () => new Array(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      L[i][j] = a[i] === b[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
  const out: Line[] = [];
  let i = 0, j = 0;
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) { out.push({ kind: 'c', old: i + 1, new: j + 1, text: a[i] }); i++; j++; }
    else if (i < a.length && (j >= b.length || L[i + 1][j] >= L[i][j + 1])) { out.push({ kind: 'd', old: i + 1, text: a[i] }); i++; }
    else { out.push({ kind: 'a', new: j + 1, text: b[j] }); j++; }
  }
  return out;
}

/** Hidden new-side lines before hunk `id` (index of its header among the file's
 *  hunks), or after the last hunk (`id` = hunk count). `from`/`to` are new line
 *  numbers, inclusive; `to` is null at the end of the file until the source is
 *  known. Inside the gap, old = new + `delta`. */
export interface Gap { id: number; from: number; to: number | null; delta: number }
/** Lines shown at each end of a gap: `down` continues the hunk above, `up` leads into the hunk below. */
export interface Reveal { up: number; down: number }

/** The gaps of a text file's diff; empty ones are left out. */
export function gaps(f: FileDiff): Gap[] {
  const out: Gap[] = [];
  let k = 0, end = 0, delta = 0;
  for (const l of f.lines) {
    if (l.kind !== 'h') continue;
    const h = hunkStart(l.text);
    if (!h) continue;
    if (h.new - 1 >= end + 1) out.push({ id: k, from: end + 1, to: h.new - 1, delta: h.old - h.new });
    end = h.new + h.newLen - 1;
    delta = h.old + h.oldLen - (h.new + h.newLen);
    k++;
  }
  if (k > 0) out.push({ id: k, from: end + 1, to: null, delta });
  return out;
}

/** The text's lines; a final newline does not start another line. */
export function sourceLines(text: string): string[] {
  const a = text.split('\n');
  if (a[a.length - 1] === '') a.pop();
  return a;
}

/** Whether `src` is the file the diff was made against: every context and added line is in place. */
export function matches(f: FileDiff, src: string[]): boolean {
  return f.lines.every((l) => (l.kind !== 'c' && l.kind !== 'a') || (l.new !== undefined && src[l.new - 1] === l.text));
}

/** The file's lines with the revealed ones merged in as context. Without `src`
 *  nothing is revealed, only the controls (`gap`) are set. */
export function expand(f: FileDiff, src: string[] | null, reveal: Record<number, Reveal>): Line[] {
  const byId = new Map(gaps(f).map((g) => [g.id, g]));
  const rows = (g: Gap, a: number, b: number): Line[] => {
    const r: Line[] = [];
    for (let n = a; n <= b; n++) r.push({ kind: 'c', old: n + g.delta, new: n, text: src![n - 1] });
    return r;
  };
  const out: Line[] = [];
  let k = 0;
  for (const l of f.lines) {
    if (l.kind !== 'h') { out.push(l); continue; }
    const g = byId.get(k);
    if (g) {
      const size = g.to! - g.from + 1;
      const r = reveal[k];
      const down = src && k > 0 ? Math.min(r?.down ?? 0, size) : 0;
      const up = src ? Math.min(r?.up ?? 0, size - down) : 0;
      const hidden = size - down - up;
      out.push(...rows(g, g.from, g.from + down - 1));
      if (hidden > 0) {
        out.push({ ...l, gap: { id: k, hidden, up: true, down: k > 0 } });
        out.push(...rows(g, g.to! - up + 1, g.to!));
      } else {
        out.push(...rows(g, g.from + down, g.to!));
      }
    } else out.push(l);
    k++;
  }
  const g = byId.get(k);
  if (g) {
    const size = src ? Math.max(0, src.length - g.from + 1) : null;
    const down = size === null ? 0 : Math.min(reveal[k]?.down ?? 0, size);
    out.push(...rows(g, g.from, g.from + down - 1));
    const hidden = size === null ? null : size - down;
    if (hidden === null || hidden > 0) out.push({ kind: 'h', text: '', gap: { id: k, hidden, up: false, down: true } });
  }
  return out;
}
