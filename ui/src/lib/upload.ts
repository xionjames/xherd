/** Quote one path for a POSIX shell: single quotes, `'` becomes `'\''` (R7). */
export const shellQuote = (s: string): string => `'${s.replace(/'/g, `'\\''`)}'`;

/** Text to paste into the terminal for stored paths: quoted, space-separated, one trailing space, no newline (R7). */
export const pastePaths = (paths: string[]): string => paths.map((p) => `${shellQuote(p)} `).join('');
