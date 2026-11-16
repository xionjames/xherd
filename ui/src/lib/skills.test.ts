import { describe, expect, it } from 'vitest';
import { NAME_HINT, fromFolder, toBase64 } from './skills';

const enc = (s: string) => new TextEncoder().encode(s);

describe('fromFolder', () => {
  it('names the skill after the folder and drops the folder from the paths', () => {
    const r = fromFolder([
      { path: 'lint-fix/SKILL.md', bytes: enc('---\nname: lint-fix\n---\n') },
      { path: 'lint-fix/ref/a.md', bytes: enc('A') },
    ]);
    expect(r.name).toBe('lint-fix');
    expect(r.files.map((f) => f.path)).toEqual(['SKILL.md', 'ref/a.md']);
    expect(r.files[1]).toEqual({ path: 'ref/a.md', text: 'A' });
    expect(r.skipped).toBe(0);
  });
  it('skips hidden files and folders and says how many', () => {
    const r = fromFolder([
      { path: 's/SKILL.md', bytes: enc('x') },
      { path: 's/.DS_Store', bytes: enc('x') },
      { path: 's/.git/config', bytes: enc('x') },
      { path: 's/a/.env', bytes: enc('x') },
    ]);
    expect(r.files.map((f) => f.path)).toEqual(['SKILL.md']);
    expect(r.skipped).toBe(3);
  });
  it('sends bytes that are not UTF-8 as base64', () => {
    const r = fromFolder([{ path: 's/bin', bytes: new Uint8Array([0xff, 0xfe, 0x00]) }]);
    expect(r.files[0]).toEqual({ path: 'bin', base64: '//4A' });
  });
  it('is empty for nothing picked', () => {
    expect(fromFolder([])).toEqual({ name: '', files: [], skipped: 0 });
  });
});

describe('toBase64', () => {
  it('handles more than one chunk', () => {
    const big = new Uint8Array(70000).fill(65);
    expect(atob(toBase64(big)).length).toBe(70000);
  });
});

describe('NAME_HINT', () => {
  it('follows the slug rule', () => {
    expect(NAME_HINT.test('lint-fix')).toBe(true);
    expect(NAME_HINT.test('Lint')).toBe(false);
    expect(NAME_HINT.test('-x')).toBe(false);
    expect(NAME_HINT.test('a'.repeat(42))).toBe(false);
  });
});
