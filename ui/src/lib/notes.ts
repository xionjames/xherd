// Inline review comments on Markdown (#25), as the change request text.
export interface MdNote { text: string; quote: string }
export const mdNoteText = (path: string, notes: Record<string, MdNote> | undefined) =>
  Object.values(notes ?? {}).map((n) => `${path}\n> ${n.quote}\n${n.text}`);

// Open questions (R-105a): the list items under an "Open questions" heading,
// as indexes into `render().blocks`. A lone "None." is no question. The daemon
// reads the same section (`xherd_flow::questions::parse`).
const heading = /^#{1,6}\s+(.*?)\s*#*\s*$/;
const item = /^([-*+]|\d+[.)])(\s|$)/;
export function questionBlocks(blocks: { raw: string }[]): number[] {
  const out: number[] = [];
  let inside = false;
  blocks.forEach((b, i) => {
    const h = heading.exec(b.raw.split('\n')[0]);
    if (h) inside = h[1].toLowerCase() === 'open questions';
    else if (inside && item.test(b.raw)) out.push(i);
  });
  const none = (i: number) => /^none\.?$/i.test(blocks[i].raw.replace(item, '').trim());
  return out.length === 1 && none(out[0]) ? [] : out;
}
