// Hash router: `#/path?query`. View state lives in the URL (spec §17.1).

function parse() {
  const h = location.hash.replace(/^#/, '') || '/';
  const [path, qs = ''] = h.split('?');
  return { path: path || '/', query: new URLSearchParams(qs) };
}

export const route = $state(parse());

addEventListener('hashchange', () => {
  const r = parse();
  route.path = r.path;
  route.query = r.query;
});

export function href(path: string, query: Record<string, string | undefined> = {}): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(query)) if (v !== undefined && v !== '') q.set(k, v);
  const s = q.toString();
  return `#${path}${s ? `?${s}` : ''}`;
}

export function go(path: string, query: Record<string, string | undefined> = {}) {
  location.hash = href(path, query);
}
