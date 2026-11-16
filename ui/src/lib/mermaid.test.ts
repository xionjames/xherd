import { describe, expect, it } from 'vitest';
import mermaid from 'mermaid';
import { themeVariables } from './mermaid';

// Token values of app.css (light at :root, dark under the dark theme).
const LIGHT: Record<string, string> = {
  '--sans': 'Inter, sans-serif', '--pen': '#ffffff', '--pen-2': '#eaeef5', '--well': '#f8fafc',
  '--fence-strong': 'rgba(18, 34, 68, .22)', '--ink': '#111a2c', '--ink-2': '#364260', '--ink-3': '#56627c',
  '--accent-wash': '#e7edfc',
};
const DARK: Record<string, string> = {
  '--sans': 'Inter, sans-serif', '--pen': '#121a2c', '--pen-2': '#1c2641', '--well': '#0d1424',
  '--fence-strong': 'rgba(160,182,235,.24)', '--ink': '#e6ebf5', '--ink-2': '#b6c0d6', '--ink-3': '#909cb6',
  '--accent-wash': 'rgba(122,156,255,.16)',
};

/** `#rrggbb` or `hsl(h, s%, l%)` as [r, g, b] in 0..255. */
function rgb(c: string): [number, number, number] {
  const s = c.trim();
  if (s.startsWith('#')) return [1, 3, 5].map((i) => parseInt(s.slice(i, i + 2), 16)) as [number, number, number];
  const m = /^hsla?\(\s*([\d.]+)\s*,\s*([\d.]+)%\s*,\s*([\d.]+)%/.exec(s);
  if (!m) throw new Error(`unreadable colour: ${c}`);
  const [h, sat, l] = [Number(m[1]), Number(m[2]) / 100, Number(m[3]) / 100];
  const a = sat * Math.min(l, 1 - l);
  const f = (n: number) => {
    const k = (n + h / 30) % 12;
    return Math.round(255 * (l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1))));
  };
  return [f(0), f(8), f(4)];
}

function luminance(c: string): number {
  const [r, g, b] = rgb(c).map((v) => {
    const x = v / 255;
    return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** The theme variables Mermaid ends up with, derived ones included. */
function resolved(tokens: Record<string, string>, dark: boolean): Record<string, string> {
  mermaid.initialize({ startOnLoad: false, theme: 'base', themeVariables: themeVariables((n) => tokens[n] ?? '', dark) });
  return mermaid.mermaidAPI.getConfig().themeVariables as Record<string, string>;
}

describe('themeVariables: ER attribute rows (#155)', () => {
  for (const [name, tokens, dark] of [['dark', DARK, true], ['light', LIGHT, false]] as const) {
    it(`keeps both ER row colours readable under the text in the ${name} theme`, () => {
      const t = resolved(tokens, dark);
      expect(t.darkMode).toBe(dark);
      expect(contrast(t.rowOdd, t.primaryTextColor)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(t.rowEven, t.primaryTextColor)).toBeGreaterThanOrEqual(4.5);
    });
  }
});
