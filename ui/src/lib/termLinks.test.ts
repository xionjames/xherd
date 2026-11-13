import { describe, expect, it } from 'vitest';
import { findUrls, type Row } from './termLinks';

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
