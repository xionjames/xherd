// Agent-written content is untrusted (spec §13): Markdown is rendered with
// marked and sanitised with DOMPurify before it reaches the page.
import { marked, type Token, type Tokens } from 'marked';
import DOMPurify from 'dompurify';
import { linkify } from './links';
import { resolveImage } from './images';

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

// Job documents use the same allow-list plus `img`. An image never keeps a
// loadable `src`: a local one is read through the daemon (`data-xh-img`), a
// remote one never loads. The original `src` is recorded by a hook, since the
// URI policy drops an unsafe one (`javascript:`, `file:`) before the rewrite
// sees it. The instance is its own so the hook does not touch the external path.
const srcOf = new WeakMap<Element, string>();
let jobPurify: ReturnType<typeof DOMPurify> | undefined;
function jobSanitizer() {
  if (!jobPurify) {
    jobPurify = DOMPurify(window);
    jobPurify.addHook('uponSanitizeAttribute', (node, data) => {
      if (node.nodeName === 'IMG' && data.attrName === 'src') srcOf.set(node, data.attrValue);
    });
  }
  return jobPurify;
}

function placeholder(doc: Document, alt: string, rest: string | Node): HTMLElement {
  const span = doc.createElement('span');
  span.className = 'imgph';
  span.append(alt ? `${alt} ` : '', rest);
  return span;
}

/** Rewrite each `img` of a sanitised fragment (R1-R4, R6); `base` is the document's directory, undefined when unknown. */
function rewriteImages(frag: DocumentFragment, base: string | undefined) {
  const doc = frag.ownerDocument;
  for (const img of Array.from(frag.querySelectorAll('img'))) {
    const src = srcOf.get(img) ?? '';
    const alt = img.getAttribute('alt') ?? '';
    const ref = base === undefined ? resolveImage(src, '') : resolveImage(src, base);
    if (ref.kind === 'data') continue;
    if (ref.kind === 'local' && base !== undefined) {
      img.removeAttribute('src');
      img.setAttribute('data-xh-img', ref.path);
      img.className = 'mdimg';
    } else if (ref.kind === 'remote' && /^https?:\/\//i.test(ref.url)) {
      const a = doc.createElement('a');
      a.href = ref.url;
      a.target = '_blank';
      a.rel = 'noopener noreferrer';
      a.textContent = ref.url;
      img.replaceWith(placeholder(doc, alt, a));
    } else {
      const reason = ref.kind === 'bad' ? ref.reason : ref.kind === 'remote' ? 'not an image' : 'not found';
      img.replaceWith(placeholder(doc, alt, ref.kind === 'remote' ? `${ref.url} — ${reason}` : `— ${reason}`));
    }
  }
}

/**
 * `added`: line texts that are new since the user last read the document;
 * every top-level block holding one of them is wrapped in `.chg`.
 * `external`: the text comes from outside (a tracker), so nothing in it loads.
 * `base`: directory of the document (see `docBase`), to resolve its local images.
 */
export function render(md: string, added?: Set<string>, external = false, base?: string): { html: string; headings: Heading[]; blocks: { html: string; raw: string }[] } {
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
  const clean = (h: string) => {
    if (external) return DOMPurify.sanitize(h, { ALLOWED_TAGS: EXTERNAL_TAGS, ALLOWED_ATTR: EXTERNAL_ATTRS, ALLOW_DATA_ATTR: false });
    const frag = jobSanitizer().sanitize(h, {
      ALLOWED_TAGS: [...EXTERNAL_TAGS, 'img'],
      ALLOWED_ATTR: [...EXTERNAL_ATTRS, 'src', 'alt', 'width', 'height'],
      ALLOW_DATA_ATTR: false,
      RETURN_DOM_FRAGMENT: true,
    });
    rewriteImages(frag, base);
    const box = frag.ownerDocument.createElement('div');
    box.append(frag);
    return box.innerHTML;
  };
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
