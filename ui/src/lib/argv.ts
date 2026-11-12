// A command typed on one line as an argv (Settings → Open in, R-706): words
// split on spaces, single or double quotes keep spaces. No variables, no
// escapes: the companion runs the argv directly, never through a shell.
export function words(s: string): string[] {
  const out: string[] = [];
  let cur = '';
  let quote: string | null = null;
  let any = false;
  for (const c of s) {
    if (quote) {
      if (c === quote) quote = null; else cur += c;
    } else if (c === '"' || c === "'") {
      quote = c;
      any = true;
    } else if (/\s/.test(c)) {
      if (any || cur) out.push(cur);
      cur = '';
      any = false;
    } else cur += c;
  }
  if (any || cur) out.push(cur);
  return out;
}

/** The argv back on one line, quoting words that need it. */
export const line = (argv: string[]): string =>
  argv.map((a) => (a === '' || /[\s'"]/.test(a) ? (a.includes('"') ? `'${a}'` : `"${a}"`) : a)).join(' ');
