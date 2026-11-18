import { describe, expect, it } from 'vitest';

// The daemon serves `fetch` only under `/api/v1` (Vite's dev proxy forwards only
// `/api` too): a literal root path such as `/healthz` is always a 404.
const sources = import.meta.glob(['../**/*.ts', '../**/*.svelte', '!../**/*.test.ts'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

const CALL = /\bfetch(?:Fn)?\(\s*/g;
// A string or template literal that starts with `/api/v1/` or `/api/v1${`.
const API_LITERAL = /^(['"`])\/api\/v1(?:\/|\$\{)/;
const IDENTIFIER = /^[A-Za-z_$][\w$]*$/;

interface FetchCall {
  file: string;
  arg: string;
  ok: boolean;
}

/** Every `fetch(` / `fetchFn(` call in the non-test sources, with its first argument. */
function fetchCalls(): FetchCall[] {
  const calls: FetchCall[] = [];
  for (const [path, text] of Object.entries(sources)) {
    const file = path.replace(/^\.\.\//, '');
    for (const m of text.matchAll(CALL)) {
      const rest = text.slice((m.index ?? 0) + m[0].length);
      const end = rest.search(/[,)]/);
      const arg = (end < 0 ? rest : rest.slice(0, end)).trim();
      calls.push({ file, arg, ok: argIsApi(text, arg) });
    }
  }
  return calls;
}

/** The argument is an /api/v1 literal, or a `const` of the same file assigned one. */
function argIsApi(text: string, arg: string): boolean {
  if (API_LITERAL.test(arg)) return true;
  if (!IDENTIFIER.test(arg)) return false;
  const name = arg.replace(/\$/g, '\\$');
  const decl = new RegExp(`\\bconst\\s+${name}\\s*(?::[^=]+)?=\\s*`).exec(text);
  return decl !== null && API_LITERAL.test(text.slice(decl.index + decl[0].length));
}

describe('UI fetch paths', () => {
  it('go through /api/v1', () => {
    const calls = fetchCalls();
    expect(calls.length).toBeGreaterThan(0);
    expect(calls.filter((c) => !c.ok).map((c) => `${c.file}: ${c.arg}`)).toEqual([]);
  });
});
