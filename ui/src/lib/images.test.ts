import { describe, expect, it } from 'vitest';
import type { FileDiff } from './diff';
import { docBase, fileBase, hasPreview, imageSides, isImage, isSvg, opensOnPreview, previewKind, reasonOf, resolveImage, stale } from './images';

const file = (over: Partial<FileDiff>): FileDiff => ({ path: 'a.png', status: 'modified', adds: 0, dels: 0, lines: [], ...over });

describe('isImage / isSvg / previewKind', () => {
  it('knows the supported extensions, case-insensitively', () => {
    expect(['a.png', 'b.JPG', 'c.jpeg', 'd.gif', 'e.webp', 'f.svg'].every(isImage)).toBe(true);
    expect(isImage('a.bmp')).toBe(false);
    expect(isImage('png')).toBe(false);
    expect(isSvg('x/y.SVG')).toBe(true);
    expect(isSvg('x.png')).toBe(false);
  });
  it('classifies previewable files', () => {
    expect(previewKind('a.md')).toBe('markdown');
    expect(previewKind('a.htm')).toBe('html');
    expect(previewKind('a.svg')).toBe('image');
    expect(previewKind('a.rs')).toBeNull();
  });
});

describe('resolveImage', () => {
  const local = (p: string) => ({ kind: 'local', path: p });
  const bad = (reason: string) => ({ kind: 'bad', reason });
  it('resolves relative paths against the base', () => {
    expect(resolveImage('a.png', 'docs')).toEqual(local('./docs/a.png'));
    expect(resolveImage('./a.png', 'docs')).toEqual(local('./docs/a.png'));
    expect(resolveImage('../a.png', 'docs/x')).toEqual(local('./docs/a.png'));
    expect(resolveImage('a.png', '')).toEqual(local('./a.png'));
  });
  it('resolves a /-rooted path against the root', () => {
    expect(resolveImage('/img/a.png', 'docs')).toEqual(local('./img/a.png'));
  });
  it('refuses a path that leaves the worktree, is empty or is not an image', () => {
    expect(resolveImage('../../a.png', 'docs')).toEqual(bad('outside the worktree'));
    expect(resolveImage('/../a.png', '')).toEqual(bad('outside the worktree'));
    expect(resolveImage('', 'docs')).toEqual(bad('not found'));
    expect(resolveImage('./', 'docs')).toEqual(bad('not found'));
    expect(resolveImage('a.txt', 'docs')).toEqual(bad('not an image'));
  });
  it('decodes percent escapes and drops the query and the fragment', () => {
    expect(resolveImage('my%20pic.png?raw=1#x', 'd')).toEqual(local('./d/my pic.png'));
    expect(resolveImage('50%.png', 'd')).toEqual(local('./d/50%.png'));
  });
  it('allows data: images of the supported types only', () => {
    expect(resolveImage('data:image/png;base64,AAAA', 'd')).toEqual({ kind: 'data' });
    expect(resolveImage('data:image/svg+xml,%3Csvg%3E', 'd')).toEqual({ kind: 'data' });
    expect(resolveImage('data:text/html,<b>', 'd')).toEqual(bad('not an image'));
    expect(resolveImage('data:image/bmp;base64,AA', 'd')).toEqual(bad('not an image'));
  });
  it('reports remote and other schemes as remote, with the URL', () => {
    expect(resolveImage('https://x.test/a.png', 'd')).toEqual({ kind: 'remote', url: 'https://x.test/a.png' });
    expect(resolveImage('//x.test/a.png', 'd')).toEqual({ kind: 'remote', url: 'https://x.test/a.png' });
    expect(resolveImage('javascript:alert(1)', 'd')).toEqual({ kind: 'remote', url: 'javascript:alert(1)' });
    expect(resolveImage('file:///etc/x', 'd')).toEqual({ kind: 'remote', url: 'file:///etc/x' });
  });
});

describe('docBase / fileBase', () => {
  it('maps a document to its directory', () => {
    expect(docBase('./a/b.md')).toBe('a');
    expect(docBase('./b.md')).toBe('');
    expect(docBase('spec.md')).toBe('.xherd/spec');
    expect(docBase('sub/x.md')).toBe('.xherd/spec/sub');
  });
  it('maps a changed file to its directory', () => {
    expect(fileBase('docs/a/b.md')).toBe('docs/a');
    expect(fileBase('b.md')).toBe('');
  });
});

describe('reasonOf', () => {
  it('turns the daemon codes into reasons', () => {
    const e = (code: string) => Object.assign(new Error('msg'), { code });
    expect(reasonOf(e('bad_path'))).toBe('outside the worktree');
    expect(reasonOf(e('no_such_doc'))).toBe('not found');
    expect(reasonOf(e('doc_missing'))).toBe('not found');
    expect(reasonOf(e('not_an_image'))).toBe('not an image');
    expect(reasonOf(e('too_large'))).toBe('too large');
    expect(reasonOf(e('other'))).toBe('msg');
    expect(reasonOf('boom')).toBe('boom');
  });
});

describe('hasPreview', () => {
  it('offers md and html unless deleted or without a diff, and images always', () => {
    expect(hasPreview(file({ path: 'a.md' }))).toBe(true);
    expect(hasPreview(file({ path: 'a.md', status: 'deleted' }))).toBe(false);
    expect(hasPreview(file({ path: 'a.html', note: 'Not shown: x' }))).toBe(false);
    expect(hasPreview(file({ path: 'a.png', status: 'deleted' }))).toBe(true);
    expect(hasPreview(file({ path: 'a.png', note: 'Not shown: x' }))).toBe(true);
    expect(hasPreview(file({ path: 'a.rs' }))).toBe(false);
  });
});

describe('imageSides', () => {
  it('picks the sides of each status', () => {
    expect(imageSides(file({ status: 'added' }))).toEqual({ after: { path: 'a.png' } });
    expect(imageSides(file({ status: 'deleted' }))).toEqual({ before: { path: 'a.png' } });
    expect(imageSides(file({}))).toEqual({ before: { path: 'a.png' }, after: { path: 'a.png' } });
    expect(imageSides(file({ path: 'b.png', status: 'renamed', oldPath: 'a.png' })))
      .toEqual({ before: { path: 'a.png' }, after: { path: 'b.png' } });
  });
  it('says too large for the new side of a file over the limit', () => {
    expect(imageSides(file({ status: 'too large', note: 'Not shown: x' }))).toEqual({ before: { path: 'a.png' }, after: 'too large' });
  });
});

describe('opensOnPreview', () => {
  it('opens a raster image on Preview, whatever the binary flag says', () => {
    expect(opensOnPreview(file({ binary: true }))).toBe(true);
    expect(opensOnPreview(file({ status: 'renamed' }))).toBe(true);
    expect(opensOnPreview(file({ path: 'a.svg' }))).toBe(false);
    expect(opensOnPreview(file({ path: 'a.md' }))).toBe(false);
    expect(opensOnPreview(file({ path: 'a.html' }))).toBe(false);
  });
});

describe('stale', () => {
  it('compares an abbreviated id with the served one by prefix', () => {
    expect(stale('abc1234', 'abc1234ffff')).toBe(false);
    expect(stale('abc1234', 'def1234ffff')).toBe(true);
  });
  it('is never stale without ids or with a zero id', () => {
    expect(stale(undefined, 'abc')).toBe(false);
    expect(stale('abc', null)).toBe(false);
    expect(stale('0000000', 'abc')).toBe(false);
    expect(stale('abc', '0000000')).toBe(false);
  });
});
