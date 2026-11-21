// Inline review comments on Markdown (#25), as the change request text.
export interface MdNote { text: string; quote: string }
export const mdNoteText = (path: string, notes: Record<string, MdNote> | undefined) =>
  Object.values(notes ?? {}).map((n) => `${path}\n> ${n.quote}\n${n.text}`);

// Open questions (R-105a): the list items under an "Open questions" heading,
// as indexes into `render().blocks`. A section whose first content is a None
// statement ("None", any case, then end, "." or ":"; text may follow) has no
// questions, whatever follows it. The daemon reads the same section
// (`xherd_flow::questions::parse`).
const heading = /^#{1,6}\s+(.*?)\s*#*\s*$/;
const item = /^([-*+]|\d+[.)])(\s|$)/;
const none = /^none(?:[.:]|$)/i;
export function questionBlocks(blocks: { raw: string }[]): number[] {
  const out: number[] = [];
  let inside = false;
  let first = true;
  let noneFirst = false;
  let start = 0;
  const close = () => { if (noneFirst) out.length = start; };
  blocks.forEach((b, i) => {
    const h = heading.exec(b.raw.split('\n')[0]);
    if (h) {
      close();
      inside = h[1].toLowerCase() === 'open questions';
      [start, first, noneFirst] = [out.length, true, false];
    } else if (inside) {
      const text = b.raw.trim();
      if (first && text) {
        first = false;
        noneFirst = none.test(text.replace(item, '').trim());
      }
      if (item.test(b.raw)) out.push(i);
    }
  });
  close();
  return out;
}
