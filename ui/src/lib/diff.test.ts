import { describe, expect, it } from 'vitest';
import { expand, filterFiles, fingerprint, gaps, hunkStart, lineDiff, matches, parseUnified, sourceLines, STEP } from './diff';

describe('parseUnified', () => {
  it('reads files, hunks and line numbers', () => {
    const d = [
      'diff --git a/src/a.rs b/src/a.rs',
      'index 1..2 100644',
      '--- a/src/a.rs',
      '+++ b/src/a.rs',
      '@@ -3,2 +3,2 @@ fn x',
      ' keep',
      '-old',
      '+new',
      'diff --git a/n.txt b/n.txt',
      'new file mode 100644',
      '@@ -0,0 +1 @@',
      '+hi',
    ].join('\n');
    const f = parseUnified(d);
    expect(f.map((x) => [x.path, x.status, x.adds, x.dels])).toEqual([
      ['src/a.rs', 'modified', 1, 1],
      ['n.txt', 'added', 1, 0],
    ]);
    expect(f[0].lines[1]).toEqual({ kind: 'c', old: 3, new: 3, text: 'keep' });
    expect(f[0].lines[3]).toEqual({ kind: 'a', new: 4, text: 'new' });
  });

  it('reads a file the daemon left out as too large', () => {
    const f = parseUnified('diff --git a/heap.hprof b/heap.hprof\nNot shown: heap.hprof is 6.0 GiB, over the 8.0 MiB limit\n');
    expect(f).toEqual([{
      path: 'heap.hprof', status: 'too large', adds: 0, dels: 0, lines: [],
      note: 'Not shown: heap.hprof is 6.0 GiB, over the 8.0 MiB limit',
    }]);
  });
});

describe('parseUnified, binary files', () => {
  const parse = (...l: string[]) => parseUnified(l.join('\n'))[0];

  it('keeps added and deleted, flags binary and reads the blob ids', () => {
    expect(parse(
      'diff --git a/img/a.png b/img/a.png', 'new file mode 100644', 'index 0000000..3f2a9c1',
      'Binary files /dev/null and b/img/a.png differ',
    )).toMatchObject({ path: 'img/a.png', status: 'added', binary: true, adds: 0, dels: 0, lines: [], newBlob: '3f2a9c1' });
    const added = parse(
      'diff --git a/img/a.png b/img/a.png', 'new file mode 100644', 'index 0000000..3f2a9c1',
      'Binary files /dev/null and b/img/a.png differ',
    );
    expect(added.oldBlob).toBeUndefined();
    const deleted = parse(
      'diff --git a/img/a.png b/img/a.png', 'deleted file mode 100644', 'index 3f2a9c1..0000000',
      'Binary files a/img/a.png and /dev/null differ',
    );
    expect(deleted).toMatchObject({ status: 'deleted', binary: true, oldBlob: '3f2a9c1' });
    expect(deleted.newBlob).toBeUndefined();
  });

  it('reads both blob ids of a modified one', () => {
    expect(parse(
      'diff --git a/a.png b/a.png', 'index 1111111..2222222 100644', 'Binary files a/a.png and b/a.png differ',
    )).toMatchObject({ status: 'modified', binary: true, oldBlob: '1111111', newBlob: '2222222' });
  });

  it('keeps the old path of a renamed one, with or without changed bytes', () => {
    expect(parse(
      'diff --git a/old.png b/new.png', 'similarity index 90%', 'rename from old.png', 'rename to new.png',
      'index 1111111..2222222', 'Binary files a/old.png and b/new.png differ',
    )).toMatchObject({ path: 'new.png', oldPath: 'old.png', status: 'renamed', binary: true, oldBlob: '1111111', newBlob: '2222222' });
    expect(parse(
      'diff --git a/old.png b/new.png', 'similarity index 100%', 'rename from old.png', 'rename to new.png',
    )).toMatchObject({ path: 'new.png', oldPath: 'old.png', status: 'renamed' });
  });

  it('gives a renamed text file its old path and no binary flag', () => {
    const f = parse('diff --git a/a.md b/b.md', 'similarity index 90%', 'rename from a.md', 'rename to b.md', '@@ -1 +1 @@', '-x', '+y');
    expect(f).toMatchObject({ path: 'b.md', oldPath: 'a.md', status: 'renamed', adds: 1, dels: 1 });
    expect(f.binary).toBeFalsy();
  });
});

describe('fingerprint', () => {
  // The hash the Changes panel used before it moved here.
  const before = (f: { lines: { kind: string; text: string }[] }) => {
    let h = 5381;
    for (const l of f.lines) for (const c of l.kind + l.text + '\n') h = ((h * 33) ^ c.charCodeAt(0)) >>> 0;
    return String(h);
  };

  it('is unchanged for a text file', () => {
    const f = parseUnified('diff --git a/a b/a\n@@ -1 +1 @@\n-x\n+y')[0];
    expect(fingerprint(f)).toBe(before(f));
  });

  it('changes with the blob ids of a binary file', () => {
    const png = (o: string, n: string) => parseUnified(
      `diff --git a/a.png b/a.png\nindex ${o}..${n} 100644\nBinary files a/a.png and b/a.png differ`,
    )[0];
    expect(fingerprint(png('1111111', '2222222'))).not.toBe(fingerprint(png('1111111', '3333333')));
    expect(fingerprint(png('1111111', '2222222'))).not.toBe(fingerprint(png('4444444', '2222222')));
    expect(fingerprint(png('1111111', '2222222'))).toBe(fingerprint(png('1111111', '2222222')));
  });
});

describe('lineDiff', () => {
  it('marks added and deleted lines', () => {
    expect(lineDiff('a\nb\nc', 'a\nx\nc').map((l) => l.kind + l.text)).toEqual(['ca', 'db', 'ax', 'cc']);
  });
});

describe('filterFiles', () => {
  const files = [{ path: 'src/Cache.rs' }, { path: 'ui/src/App.svelte' }, { path: 'README.md' }];
  it('keeps every file for an empty or blank query', () => {
    expect(filterFiles(files, '')).toEqual(files);
    expect(filterFiles(files, '  ')).toEqual(files);
  });
  it('matches a substring of the path, ignoring case', () => {
    expect(filterFiles(files, 'cache').map((f) => f.path)).toEqual(['src/Cache.rs']);
    expect(filterFiles(files, ' SRC/ ').map((f) => f.path)).toEqual(['src/Cache.rs', 'ui/src/App.svelte']);
    expect(filterFiles(files, 'nope')).toEqual([]);
  });
});

// A 100-line file l1..l100 where one line is replaced by two (X11, Y11) and Z
// is added after l52: two hunks, 102 lines in the new file.
const old100 = Array.from({ length: 100 }, (_, i) => `l${i + 1}`);
const new102 = [...old100.slice(0, 10), 'X11', 'Y11', ...old100.slice(11, 52), 'Z', ...old100.slice(52)];
const two = parseUnified([
  'diff --git a/f.txt b/f.txt',
  '--- a/f.txt',
  '+++ b/f.txt',
  '@@ -8,7 +8,8 @@ head one',
  ' l8', ' l9', ' l10', '-l11', '+X11', '+Y11', ' l12', ' l13', ' l14',
  '@@ -50,6 +51,7 @@ head two',
  ' l50', ' l51', ' l52', '+Z', ' l53', ' l54', ' l55',
].join('\n'))[0];
const nums = (ls: { old?: number; new?: number; kind: string }[]) => ls.filter((l) => l.kind === 'c').map((l) => [l.old, l.new]);

describe('hunkStart', () => {
  it('reads both sides, an empty side starting after the named line', () => {
    expect(hunkStart('@@ -3,2 +4 @@ fn x')).toEqual({ old: 3, new: 4, oldLen: 2, newLen: 1 });
    expect(hunkStart('@@ -5,2 +4,0 @@')).toEqual({ old: 5, new: 5, oldLen: 2, newLen: 0 });
    expect(hunkStart('not a header')).toBeNull();
  });
});

describe('gaps', () => {
  it('finds the first gap, a middle gap with a shifted numbering and the open end', () => {
    expect(gaps(two)).toEqual([
      { id: 0, from: 1, to: 7, delta: 0 },
      { id: 1, from: 16, to: 50, delta: -1 },
      { id: 2, from: 58, to: null, delta: -2 },
    ]);
  });
  it('leaves out an empty first gap and has none without hunks', () => {
    const f = parseUnified('diff --git a/a b/a\n@@ -1,2 +1,3 @@\n a\n+b\n c')[0];
    expect(gaps(f).map((g) => g.id)).toEqual([1]);
    const r = parseUnified('diff --git a/a b/b\nrename from a\nrename to b\n')[0];
    expect(gaps(r)).toEqual([]);
  });
});

describe('sourceLines', () => {
  it('drops one final newline only', () => {
    expect(sourceLines('a\nb\n')).toEqual(['a', 'b']);
    expect(sourceLines('a\nb')).toEqual(['a', 'b']);
    expect(sourceLines('a\n\n')).toEqual(['a', '']);
    expect(sourceLines('')).toEqual([]);
  });
});

describe('matches', () => {
  it('is true for the file the diff was made against and false once it moved on', () => {
    expect(matches(two, new102)).toBe(true);
    expect(matches(two, [...new102.slice(0, 9), 'changed', ...new102.slice(10)])).toBe(false);
    expect(matches(two, new102.slice(0, 20))).toBe(false);
  });
});

describe('expand', () => {
  it('sets only the controls without a source', () => {
    const out = expand(two, null, {});
    expect(out.length).toBe(two.lines.length + 1);
    expect(out.filter((l) => l.kind === 'h').map((l) => l.gap)).toEqual([
      { id: 0, hidden: 7, up: true, down: false },
      { id: 1, hidden: 35, up: true, down: true },
      { id: 2, hidden: null, up: false, down: true },
    ]);
    expect(out[out.length - 1].text).toBe('');
  });

  it('reveals up from a hunk, numbered on both sides', () => {
    const out = expand(two, new102, { 1: { up: STEP, down: 0 } });
    const h = out.findIndex((l) => l.text.startsWith('@@ -50'));
    expect(out[h].gap).toEqual({ id: 1, hidden: 15, up: true, down: true });
    expect(nums(out.slice(h + 1, h + 1 + STEP))[0]).toEqual([50 - 1 - 19, 31]);
    expect(out[h + STEP].text).toBe('l49');
    expect(out[h + STEP].new).toBe(50);
    expect(out[h + STEP].old).toBe(49);
  });

  it('reveals down from the hunk above', () => {
    const out = expand(two, new102, { 1: { up: 0, down: 5 } });
    const h = out.findIndex((l) => l.text.startsWith('@@ -50'));
    expect(out.slice(h - 5, h).map((l) => [l.old, l.new, l.text])).toEqual([
      [15, 16, 'l15'], [16, 17, 'l16'], [17, 18, 'l17'], [18, 19, 'l18'], [19, 20, 'l19'],
    ]);
    expect(out[h].gap?.hidden).toBe(30);
  });

  it('joins the hunks when both ends meet and drops the header', () => {
    const out = expand(two, new102, { 1: { up: 20, down: 15 } });
    expect(out.some((l) => l.text.startsWith('@@ -50'))).toBe(false);
    const c = out.filter((l) => l.kind === 'c').map((l) => l.new);
    expect(c.slice(c.indexOf(15), c.indexOf(51) + 1)).toEqual(Array.from({ length: 37 }, (_, i) => 15 + i));
    expect(expand(two, new102, { 1: { up: 100, down: 0 } }).some((l) => l.text.startsWith('@@ -50'))).toBe(false);
  });

  it('reveals the first gap with nothing above it, and keeps the header of a full one', () => {
    const out = expand(two, new102, { 0: { up: 100, down: 100 } });
    expect(out[0]).toEqual({ kind: 'c', old: 1, new: 1, text: 'l1' });
    expect(out.filter((l) => l.kind === 'h' && l.text.startsWith('@@ -8'))).toHaveLength(0);
    expect(expand(two, new102, {})[0].gap).toEqual({ id: 0, hidden: 7, up: true, down: false });
  });

  it('reveals to the end of the file and then drops the trailing row', () => {
    const part = expand(two, new102, { 2: { up: 0, down: STEP } });
    const tail = part[part.length - 1];
    expect(tail.gap).toEqual({ id: 2, hidden: 102 - 57 - STEP, up: false, down: true });
    const rows = part.slice(part.length - 1 - STEP, -1);
    expect(rows[0]).toEqual({ kind: 'c', old: 56, new: 58, text: 'l56' });
    const all = expand(two, new102, { 2: { up: 0, down: 1000 } });
    const last = all[all.length - 1];
    expect(last).toEqual({ kind: 'c', old: 100, new: 102, text: 'l100' });
    expect(all.filter((l) => l.kind === 'h')).toHaveLength(2);
  });

  it('keeps the original lines in order', () => {
    const out = expand(two, new102, { 0: { up: 3, down: 0 }, 1: { up: 2, down: 2 } });
    expect(out.filter((l) => l.kind !== 'c' || two.lines.includes(l)).map((l) => l.text).filter((t) => t.startsWith('@@'))).toEqual(['@@ -8,7 +8,8 @@ head one', '@@ -50,6 +51,7 @@ head two']);
    expect(out.filter((l) => l.kind === 'a').map((l) => l.text)).toEqual(['X11', 'Y11', 'Z']);
  });
});
