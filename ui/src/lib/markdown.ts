// Agent-written content is untrusted (spec §13): Markdown is rendered with
// marked and sanitised with DOMPurify before it reaches the page.
import { marked, type Token, type Tokens } from 'marked';
import DOMPurify from 'dompurify';
import { linkify } from './links';

export interface Heading { id: string; text: string; level: number }

/** Split off YAML frontmatter: `key: value` pairs shown as pills. */
export function frontmatter(text: string): { meta: [string, string][]; body: string } {
  const m = text.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?/);
  if (!m) return { meta: [], body: text };
  const meta = m[1]
    .split(/\r?\n/)
    .map((l) => l.match(/^([\w.-]+):\s*(.*)$/))
    .filter((x): x is RegExpMatchArray => !!x)
    .map((x) => [x[1], x[2]] as [string, string]);
  return { meta, body: text.slice(m[0].length) };
}

function slug(s: string) {
  return s.toLowerCase().replace(/[^\w]+/g, '-').replace(/^-|-$/g, '');
}

// Text written by third parties (a ticket and its comments) may not make the
// browser fetch anything by itself: an image or media URL would tell its owner
// who read it and when. Links still work, since they need a click.
// An allow-list, not a denylist: DOMPurify keeps `background="https://…"` on a
// table cell and SVG `feImage`, which fetch by themselves. Only what Markdown
// produces is kept, with no SVG, no MathML and no presentational attributes
// that take a URL.
const EXTERNAL_TAGS = ['p', 'a', 'ul', 'ol', 'li', 'code', 'pre', 'blockquote', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
  'table', 'thead', 'tbody', 'tfoot', 'tr', 'th', 'td', 'em', 'strong', 'b', 'i', 'del', 's', 'sub', 'sup', 'br', 'hr', 'span', 'div'];
const EXTERNAL_ATTRS = ['href', 'title', 'align', 'start', 'id', 'class', 'target', 'rel'];

/**
 * `added`: line texts that are new since the user last read the document;
 * every top-level block holding one of them is wrapped in `.chg`.
 * `external`: the text comes from outside (a tracker), so nothing in it loads.
 */
export function render(md: string, added?: Set<string>, external = false): { html: string; headings: Heading[]; blocks: { html: string; raw: string }[] } {
  const headings: Heading[] = [];
  const renderer = new marked.Renderer();
  renderer.heading = ({ tokens, depth }) => {
    const text = tokens.map((t) => ('text' in t ? t.text : t.raw)).join('');
    let id = `h-${slug(text)}`;
    while (headings.some((h) => h.id === id)) id += '-';
    headings.push({ id, text, level: depth });
    return `<h${depth} id="${id}">${marked.parseInline(text)}</h${depth}>`;
  };
  // Top-level blocks, each sanitised on its own: the page can comment on one.
  // `target` is kept for the links `linkify` adds (it runs before this, see below).
  const clean = (h: string) =>
    external
      ? DOMPurify.sanitize(h, { ALLOWED_TAGS: EXTERNAL_TAGS, ALLOWED_ATTR: EXTERNAL_ATTRS, ALLOW_DATA_ATTR: false })
      : DOMPurify.sanitize(h, { FORBID_TAGS: ['style', 'form', 'input'], FORBID_ATTR: ['style'], ADD_ATTR: ['target'] });
  const tokens = marked.lexer(md, { gfm: true });
  // A list is split into one block per item (a nested list stays in its item),
  // so each item can be commented on; an ordered list keeps its numbers.
  const parts: Token[] = tokens
    .filter((t) => t.type !== 'space')
    .flatMap((t) =>
      t.type === 'list' && t.items.length > 1
        ? t.items.map((item: Tokens.ListItem, i: number) => ({
            ...t,
            items: [item],
            raw: item.raw,
            start: t.ordered ? (Number(t.start) || 1) + i : t.start,
          }))
        : [t],
    );
  const blocks = parts.map((t) => {
    const part = Object.assign([t], { links: tokens.links });
    let html = marked.parser(part, { renderer, gfm: true });
    if (added?.size && t.raw.split('\n').some((l) => l.trim() && added.has(l.trim()))) html = `<div class="chg">${html}</div>`;
    // Sanitise last: linkify parses and serialises the HTML again, which DOMPurify must see.
    return { html: clean(linkify(html)), raw: t.raw.trim() };
  });
  const html = blocks.map((b) => b.html).join('');
  return { html, headings, blocks };
}
