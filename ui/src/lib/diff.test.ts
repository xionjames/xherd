import { describe, expect, it } from 'vitest';
import { lineDiff, parseUnified } from './diff';

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
});

describe('lineDiff', () => {
  it('marks added and deleted lines', () => {
    expect(lineDiff('a\nb\nc', 'a\nx\nc').map((l) => l.kind + l.text)).toEqual(['ca', 'db', 'ax', 'cc']);
  });
});
