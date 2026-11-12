import { describe, expect, it } from 'vitest';
import { questionBlocks } from './notes';

// Blocks as `render()` splits them: one per heading, paragraph or list item.
const q = (...raws: string[]) => {
  const blocks = raws.map((raw) => ({ raw }));
  return questionBlocks(blocks).map((i) => raws[i]);
};

describe('questionBlocks', () => {
  it('finds the items under the heading', () => {
    expect(q('# Spec', '## Requirements', '- R1', '- R2', '## Open questions', '- Which db?', '- Who owns it?\n  - nested', 'A note.', '## Assumptions', '- a'))
      .toEqual(['- Which db?', '- Who owns it?\n  - nested']);
  });
  it('one question, ordered items, any level and case', () => {
    expect(q('### open questions ##', '1. Only one?')).toEqual(['1. Only one?']);
  });
  it('none, empty or missing', () => {
    expect(q('## Open questions', '- None.')).toEqual([]);
    expect(q('## Open questions', '- none')).toEqual([]);
    expect(q('## Open questions', 'None.', '## Assumptions', '- a')).toEqual([]);
    expect(q('## Approach', '- a')).toEqual([]);
    expect(q('## Open questions', '- a?', '- None.')).toEqual(['- a?', '- None.']);
  });
});
