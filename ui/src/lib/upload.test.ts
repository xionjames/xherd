import { describe, expect, it } from 'vitest';
import { pastePaths, shellQuote } from './upload';

describe('shellQuote', () => {
  it('wraps a plain path in single quotes', () => {
    expect(shellQuote('/w/.xherd/uploads/shot.png')).toBe("'/w/.xherd/uploads/shot.png'");
  });

  it('closes, escapes and reopens around a single quote', () => {
    expect(shellQuote("it's")).toBe("'it'\\''s'");
  });

  it('keeps spaces, dollars and backticks inert', () => {
    expect(shellQuote('/a b/$HOME/`x`')).toBe("'/a b/$HOME/`x`'");
  });
});

describe('pastePaths', () => {
  it('quotes each path, separates with a space and ends with one space', () => {
    expect(pastePaths(['/w/a.png', "/w/b'c.png"])).toBe("'/w/a.png' '/w/b'\\''c.png' ");
  });

  it('never ends with a newline', () => {
    expect(pastePaths(['/w/a\nb.png'])).not.toMatch(/\n$/);
    expect(pastePaths(['/w/a.png'])).toBe("'/w/a.png' ");
  });

  it('gives an empty string for no paths', () => {
    expect(pastePaths([])).toBe('');
  });
});
