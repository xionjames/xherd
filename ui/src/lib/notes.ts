// Inline review comments on Markdown (#25), as the change request text.
export interface MdNote { text: string; quote: string }
export const mdNoteText = (path: string, notes: Record<string, MdNote> | undefined) =>
  Object.values(notes ?? {}).map((n) => `${path}\n> ${n.quote}\n${n.text}`);
