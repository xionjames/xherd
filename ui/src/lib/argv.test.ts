import { describe, expect, it } from 'vitest';
import { line, words } from './argv';

describe('argv', () => {
  it('splits on spaces and keeps quoted spaces', () => {
    expect(words('idea  {dir}')).toEqual(['idea', '{dir}']);
    expect(words(`open -a "Sublime Text" '{dir}'`)).toEqual(['open', '-a', 'Sublime Text', '{dir}']);
    expect(words('x ""')).toEqual(['x', '']);
    expect(words('  ')).toEqual([]);
  });
  it('round-trips through line', () => {
    for (const a of [['open', '-a', 'Sublime Text', '{dir}'], ['say', 'it"s'], ['x', '']]) {
      expect(words(line(a))).toEqual(a);
    }
  });
});
