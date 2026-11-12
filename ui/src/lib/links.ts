// Link rules from the tracker plugins (`data.tracker.links`): text that matches
// a pattern (a ticket key, `owner/repo#12`) becomes a link. The patterns were
// checked by the daemon (linear-time subset, no nested repeats); the URL is a
// template whose `{0}`…`{9}` are the match and its groups, percent-encoded.
// A link is only made when the final URL is https.

export interface LinkRule { plugin: string; pattern: string; url: string }
export type Seg = { text: string; href?: string };

const MAX_TEXT = 100_000;
const MAX_LINKS = 200;

let compiled: { re: RegExp; url: string }[] = [];

export function setLinkRules(rules: LinkRule[]) {
  compiled = rules.flatMap((r) => {
    try { return [{ re: new RegExp(r.pattern, 'g'), url: r.url }]; } catch { return []; }
  });
}

function href(m: RegExpMatchArray, url: string): string | null {
  const out = url.replace(/\{(\d)\}/g, (_, n) => encodeURIComponent(m[Number(n)] ?? ''));
  return out.startsWith('https://') ? out : null;
}

/** `text` cut into plain and linked pieces; the earliest match wins, then the earlier rule. */
export function segments(text: string): Seg[] {
  if (!compiled.length || !text || text.length > MAX_TEXT) return [{ text }];
  const found: { start: number; end: number; href: string }[] = [];
  for (const { re, url } of compiled) {
    for (const m of text.matchAll(re)) {
      const h = m[0] ? href(m, url) : null;
      if (h && m.index !== undefined) found.push({ start: m.index, end: m.index + m[0].length, href: h });
      if (found.length >= MAX_LINKS) break;
    }
  }
  found.sort((a, b) => a.start - b.start);
  const out: Seg[] = [];
  let at = 0;
  for (const f of found) {
    if (f.start < at) continue;
    if (f.start > at) out.push({ text: text.slice(at, f.start) });
    out.push({ text: text.slice(f.start, f.end), href: f.href });
    at = f.end;
  }
  if (at < text.length) out.push({ text: text.slice(at) });
  return out.length ? out : [{ text }];
}

/** The link a whole string stands for (an agent's ticket key), if a rule matches all of it. */
export function linkFor(key: string): string | null {
  const s = segments(key);
  return s.length === 1 && s[0].href ? s[0].href : null;
}

/** Links in already sanitised HTML: text nodes outside `a`, `code` and `pre`. */
export function linkify(html: string): string {
  if (!compiled.length || typeof DOMParser === 'undefined') return html;
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  const skip = new Set(['A', 'CODE', 'PRE', 'SCRIPT', 'STYLE']);
  const walk = (node: Node) => {
    for (const child of Array.from(node.childNodes)) {
      if (child.nodeType === Node.TEXT_NODE) {
        const segs = segments(child.textContent ?? '');
        if (segs.some((s) => s.href)) {
          const frag = doc.createDocumentFragment();
          for (const s of segs) {
            if (!s.href) { frag.append(s.text); continue; }
            const a = doc.createElement('a');
            a.href = s.href;
            a.target = '_blank';
            a.rel = 'noopener noreferrer nofollow';
            a.textContent = s.text;
            frag.append(a);
          }
          child.replaceWith(frag);
        }
      } else if (child.nodeType === Node.ELEMENT_NODE && !skip.has((child as Element).tagName)) {
        walk(child);
      }
    }
  };
  walk(doc.body);
  return doc.body.innerHTML;
}
