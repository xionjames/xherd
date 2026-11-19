import { describe, expect, it } from 'vitest';
import { findPaths, findUrls, pathLinks, pathUnder, type Row } from './termLinks';

const rows = (...t: (string | [string, boolean])[]): Row[] =>
  t.map((x) => (typeof x === 'string' ? { text: x, wrapped: false } : { text: x[0], wrapped: x[1] }));
const pad = (s: string, n: number) => s.padEnd(n);

describe('findUrls', () => {
  it('finds a URL on one row and drops trailing punctuation', () => {
    expect(findUrls(rows('see https://a.example/x/y. ok'), 80)).toEqual([
      { url: 'https://a.example/x/y', segs: [{ row: 0, start: 4, end: 25 }] },
    ]);
  });

  it('joins a URL that xterm wrapped', () => {
    const f = findUrls(rows('xx https://a.example/lo', ['ng/path more', true]), 23);
    expect(f[0].url).toBe('https://a.example/long/path');
    expect(f[0].segs).toEqual([{ row: 0, start: 3, end: 23 }, { row: 1, start: 0, end: 7 }]);
  });

  it('joins a URL the TUI broke near the edge and indented (Claude Code)', () => {
    // 100 columns; the row ends 8 columns early, the rest is indented by 2.
    const f = findUrls(
      rows(
        pad('  Original prompt: see https://example.com/this-is-a-very-long-path/with/many/segments/and-m', 100),
        '  ore-segments/to-force-a-wrap?query=1&b=2 ok',
      ),
      100,
    );
    expect(f).toHaveLength(1);
    expect(f[0].url).toBe('https://example.com/this-is-a-very-long-path/with/many/segments/and-more-segments/to-force-a-wrap?query=1&b=2');
    expect(f[0].segs[1]).toEqual({ row: 1, start: 2, end: 42 });
  });

  it('joins over three rows', () => {
    const f = findUrls(rows(pad('https://a.example/' + 'x'.repeat(70), 80), '  ' + 'y'.repeat(72), '  zz end'), 80);
    expect(f[0].url).toBe('https://a.example/' + 'x'.repeat(70) + 'y'.repeat(72) + 'zz');
    expect(f[0].segs.map((s) => s.row)).toEqual([0, 1, 2]);
  });

  it('does not join a short row to the next one', () => {
    const f = findUrls(rows('  see https://a.example/x', '  and then more text'), 100);
    expect(f.map((x) => x.url)).toEqual(['https://a.example/x']);
  });

  it('keeps looking after a joined URL', () => {
    const f = findUrls(rows(pad('https://a.example/' + 'x'.repeat(70), 80), '  yy https://b.example/z'), 80);
    expect(f.map((x) => x.url)).toEqual(['https://a.example/' + 'x'.repeat(70) + 'yy', 'https://b.example/z']);
  });
});

describe('pathUnder', () => {
  const w = '/ws/worktrees/app/job';
  it('accepts paths inside the workdir and cleans them', () => {
    expect(pathUnder(w, w + '/.xherd/uploads/shot.png')).toBe(w + '/.xherd/uploads/shot.png');
    expect(pathUnder(w + '/', w + '//a/./b')).toBe(w + '/a/b');
    expect(pathUnder(w, w + '/a/../b')).toBe(w + '/b');
  });
  it('refuses outside, the workdir itself, `..` escapes, a sibling prefix and relative paths', () => {
    expect(pathUnder(w, '/etc/passwd')).toBeNull();
    expect(pathUnder(w, w)).toBeNull();
    expect(pathUnder(w, w + '/a/../../x')).toBeNull();
    expect(pathUnder(w, w + '/../other/x')).toBeNull();
    expect(pathUnder(w, w + '-two/x')).toBeNull();
    expect(pathUnder(w, 'a/b')).toBeNull();
    expect(pathUnder('rel', '/rel/x')).toBeNull();
  });
});

describe('findPaths', () => {
  const w = '/ws/job';
  it('finds a path under the workdir, not one outside', () => {
    const f = findPaths(rows('saved /ws/job/a/b.png and /etc/hosts'), 80, w);
    expect(f).toEqual([{ url: '/ws/job/a/b.png', segs: [{ row: 0, start: 6, end: 21 }] }]);
  });
  it('drops trailing punctuation, takes quoted and bracketed paths, cleans `..`', () => {
    const f = findPaths(rows('see "/ws/job/x/../y.txt", (/ws/job/z.md). done /ws/job/w.'), 80, w);
    expect(f.map((x) => x.url)).toEqual(['/ws/job/y.txt', '/ws/job/z.md', '/ws/job/w']);
  });
  it('is not fooled by a URL, or by a path glued to a word', () => {
    expect(findPaths(rows('https://h.example/ws/job/a x/ws/job/b'), 80, w)).toEqual([]);
  });
  it('joins a path xterm wrapped or the TUI broke', () => {
    const wrapped = findPaths(rows('at /ws/job/aaaa', ['bb/c.png ok', true]), 15, w);
    expect(wrapped[0].url).toBe('/ws/job/aaaabb/c.png');
    expect(wrapped[0].segs).toEqual([{ row: 0, start: 3, end: 15 }, { row: 1, start: 0, end: 8 }]);
    const broken = findPaths(rows(pad('  /ws/job/' + 'x'.repeat(78), 100), '  yy.txt tail'), 100, w);
    expect(broken[0].url).toBe('/ws/job/' + 'x'.repeat(78) + 'yy.txt');
  });
  it('leaves findUrls alone on the same rows', () => {
    expect(findUrls(rows('/ws/job/a https://a.example/x'), 80).map((x) => x.url)).toEqual(['https://a.example/x']);
  });
});

describe('pathLinks on a real terminal', () => {
  it('links the path to the clean path', async () => {
    const { Terminal } = await import('@xterm/xterm');
    const term = new Terminal({ cols: 60, rows: 5, allowProposedApi: true });
    await new Promise<void>((done) => term.write('wrote /ws/job/./a/b.png.\r\n', done));
    const links = await new Promise<{ text: string; range: { start: { x: number }; end: { x: number } } }[]>(
      (done) => pathLinks(term, '/ws/job', () => {}).provideLinks(1, (l) => done((l ?? []) as never)),
    );
    expect(links).toHaveLength(1);
    expect(links[0].text).toBe('/ws/job/a/b.png');
    expect([links[0].range.start.x, links[0].range.end.x]).toEqual([7, 23]);
  });
});

describe('urlLinks on a real terminal', () => {
  it('links both rows of a URL Claude Code broke to the whole URL', async () => {
    const { Terminal } = await import('@xterm/xterm');
    const { urlLinks } = await import('./termLinks');
    const term = new Terminal({ cols: 100, rows: 10, allowProposedApi: true });
    const url = 'https://example.com/this-is-a-very-long-path/with/many/segments/and-more-segments/to-force-a-wrap?query=1&b=2';
    // What Claude Code printed at 100 columns: ⏺ is one cell, the row ends
    // 8 columns early and the rest is indented.
    await new Promise<void>((done) =>
      term.write('⏺ see ' + url.slice(0, 84) + '\r\n  ' + url.slice(84) + ' ok\r\n', done),
    );
    const at = (y: number) =>
      new Promise<{ text: string; range: { start: { x: number }; end: { x: number } } }[]>((done) =>
        urlLinks(term, () => {}).provideLinks(y, (l) => done((l ?? []) as never)),
      );
    const [a] = await at(1);
    const [b] = await at(2);
    expect(a.text).toBe(url);
    expect(b.text).toBe(url);
    expect([a.range.start.x, a.range.end.x]).toEqual([7, 90]);
    expect([b.range.start.x, b.range.end.x]).toEqual([3, 3 + url.length - 84 - 1]);
  });
});
