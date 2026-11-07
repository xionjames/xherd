// Unified diff parsing for the Changes tab, and a small line diff for
// "changed since you approved".

export interface Line { kind: 'a' | 'd' | 'c' | 'h'; old?: number; new?: number; text: string }
export interface FileDiff { path: string; status: 'added' | 'deleted' | 'modified' | 'renamed' | 'binary'; adds: number; dels: number; lines: Line[] }

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
