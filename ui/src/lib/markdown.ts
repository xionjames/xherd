// Agent-written content is untrusted (spec §13): Markdown is rendered with
// marked and sanitised with DOMPurify before it reaches the page.
import { marked } from 'marked';
import DOMPurify from 'dompurify';

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

export function render(md: string): { html: string; headings: Heading[] } {
  const headings: Heading[] = [];
  const renderer = new marked.Renderer();
  renderer.heading = ({ tokens, depth }) => {
    const text = tokens.map((t) => ('text' in t ? t.text : t.raw)).join('');
    let id = `h-${slug(text)}`;
    while (headings.some((h) => h.id === id)) id += '-';
    headings.push({ id, text, level: depth });
    return `<h${depth} id="${id}">${marked.parseInline(text)}</h${depth}>`;
  };
  const raw = marked.parse(md, { renderer, async: false, gfm: true }) as string;
  const html = DOMPurify.sanitize(raw, { FORBID_TAGS: ['style', 'form', 'input'], FORBID_ATTR: ['style'] });
  return { html, headings };
}
