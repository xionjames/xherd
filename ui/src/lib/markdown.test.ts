// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { render } from './markdown';

const html = (md: string, base: string | undefined = '.xherd/spec') => render(md, undefined, false, base).html;
const dom = (h: string) => {
  const d = document.createElement('div');
  d.innerHTML = h;
  return d;
};

describe('images in job Markdown', () => {
  it('rewrites a Markdown image to data-xh-img with no src', () => {
    const img = dom(html('![shot](a.png)')).querySelector('img')!;
    expect(img.getAttribute('data-xh-img')).toBe('./.xherd/spec/a.png');
    expect(img.hasAttribute('src')).toBe(false);
    expect(img.getAttribute('alt')).toBe('shot');
    expect(img.className).toBe('mdimg');
  });

  it('rewrites a raw <img> the same way', () => {
    const img = dom(html('<img src="sub/b.png" alt="b">')).querySelector('img')!;
    expect(img.getAttribute('data-xh-img')).toBe('./.xherd/spec/sub/b.png');
    expect(img.hasAttribute('src')).toBe(false);
  });

  it('resolves against the document directory', () => {
    expect(dom(html('![x](../a.png)', 'docs/guide')).querySelector('img')!.getAttribute('data-xh-img')).toBe('./docs/a.png');
    expect(dom(html('![x](/a.png)', 'docs/guide')).querySelector('img')!.getAttribute('data-xh-img')).toBe('./a.png');
  });

  it('keeps a supported data: image', () => {
    const img = dom(html('![x](data:image/png;base64,AAAA)')).querySelector('img')!;
    expect(img.getAttribute('src')).toBe('data:image/png;base64,AAAA');
    expect(img.hasAttribute('data-xh-img')).toBe(false);
  });

  it('never loads a remote image: a link placeholder instead', () => {
    const d = dom(html('![logo](https://example.com/x.png)'));
    expect(d.querySelector('img')).toBeNull();
    const a = d.querySelector('.imgph a')!;
    expect(a.getAttribute('href')).toBe('https://example.com/x.png');
    expect(a.getAttribute('target')).toBe('_blank');
    expect(a.getAttribute('rel')).toBe('noopener noreferrer');
    expect(d.querySelector('.imgph')!.textContent).toContain('logo');
  });

  it('treats //host as remote', () => {
    const d = dom(html('<img src="//example.com/x.png">'));
    expect(d.querySelector('img')).toBeNull();
    expect(d.querySelector('.imgph a')!.getAttribute('href')).toBe('https://example.com/x.png');
  });

  it('shows javascript: and file: sources as text, not as links', () => {
    for (const src of ['javascript:alert(1)', 'file:///etc/passwd']) {
      const d = dom(html(`<img src="${src}" alt="z">`));
      expect(d.querySelector('img')).toBeNull();
      expect(d.querySelector('a')).toBeNull();
      expect(d.querySelector('.imgph')!.textContent).toContain(src);
    }
    const d = dom(html('![x](javascript:alert(1))'));
    expect(d.querySelector('img')).toBeNull();
    expect(d.querySelector('a')).toBeNull();
  });

  it('refuses sources outside the worktree or that are not images', () => {
    expect(dom(html('![x](../../../a.png)')).querySelector('.imgph')!.textContent).toContain('outside the worktree');
    expect(dom(html('![x](notes.txt)')).querySelector('.imgph')!.textContent).toContain('not an image');
    expect(dom(html('![x](data:text/html;base64,AAAA)')).querySelector('.imgph')!.textContent).toContain('not an image');
  });

  it('refuses local images when the base is unknown', () => {
    const d = dom(render('![x](a.png)').html);
    expect(d.querySelector('img')).toBeNull();
    expect(d.querySelector('.imgph')!.textContent).toContain('not found');
  });

  it('strips what fetches by itself', () => {
    const out = html(
      [
        '<img srcset="https://e.com/a.png 1x" src="a.png">',
        '<picture><source srcset="https://e.com/b.png"><img src="b.png"></picture>',
        '<video poster="https://e.com/p.png" src="https://e.com/v.mp4"></video>',
        '<svg><image href="https://e.com/c.png"/></svg>',
        '<table><tr><td background="https://e.com/d.png">x</td></tr></table>',
        '<iframe src="https://e.com/f"></iframe>',
      ].join('\n\n'),
    );
    for (const bad of ['srcset', '<source', 'poster', '<video', '<svg', '<image', 'background', '<iframe', 'e.com']) {
      expect(out).not.toContain(bad);
    }
    expect(dom(out).querySelectorAll('img[data-xh-img]').length).toBe(2);
  });

  it('keeps the rewrite on a block that linkify touched, and the other output unchanged', () => {
    const r = render('# Title\n\n- one ![x](a.png)\n- two\n\n```mermaid\ngraph TD\n```\n', undefined, false, '');
    expect(r.headings.map((h) => h.text)).toEqual(['Title']);
    expect(r.blocks.length).toBe(4);
    expect(r.html).toContain('data-xh-img="./a.png"');
    expect(r.html).toContain('<code class="language-mermaid">');
  });

  it('leaves the external path as it was: no images at all', () => {
    const out = render('![x](a.png) <img src="https://e.com/a.png">', undefined, true).html;
    expect(out).not.toContain('<img');
    expect(out).not.toContain('data-xh-img');
  });
});
