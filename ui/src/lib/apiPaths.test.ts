import { describe, expect, it } from 'vitest';

// The daemon serves `fetch` only under `/api/v1` (Vite's dev proxy forwards only
// `/api` too): a literal root path such as `/healthz` is always a 404.
const sources = import.meta.glob(['../**/*.ts', '../**/*.svelte', '!../**/*.test.ts'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

/** Literal URLs starting with `/` passed to `fetch(`, as `file: url`. */
function fetchUrls(): string[] {
  const found: string[] = [];
  const call = /\bfetch\(\s*(['"`])(\/[^'"`]*)\1?/g;
  for (const [path, text] of Object.entries(sources)) {
    for (const m of text.matchAll(call)) found.push(`${path.replace(/^\.\.\//, '')}: ${m[2]}`);
  }
  return found.sort();
}

describe('UI fetch paths', () => {
  it('go through /api/v1', () => {
    const urls = fetchUrls();
    expect(urls.length).toBeGreaterThan(0);
    // A template literal may continue the prefix with `${path}` (api.ts).
    const outside = urls.filter((u) => !/: \/api\/v1(\/|\$\{)/.test(u));
    expect(outside).toEqual([]);
  });
});
