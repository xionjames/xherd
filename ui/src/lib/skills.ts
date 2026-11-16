// The skills library (R25): turning a picked folder into the files `POST /skills` takes. Pure.

/** A file the browser read: its path inside the picked folder (`folder/a/b.md`) and bytes. */
export interface Picked { path: string; bytes: Uint8Array }

export interface SkillFilePost { path: string; text?: string; base64?: string }

const decoder = new TextDecoder('utf-8', { fatal: true });

/** Base64 of the bytes, in chunks so a large file does not overflow the call stack. */
export function toBase64(bytes: Uint8Array): string {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

/** UTF-8 text goes as `text`, anything else as `base64`. */
function body(bytes: Uint8Array): { text: string } | { base64: string } {
  try {
    return { text: decoder.decode(bytes) };
  } catch {
    return { base64: toBase64(bytes) };
  }
}

/**
 * The folder's name and its files without the first path part. Hidden files
 * and folders (`.git`, `.DS_Store`) are left out, and counted: the library does not keep them.
 */
export function fromFolder(picked: Picked[]): { name: string; files: SkillFilePost[]; skipped: number } {
  const name = picked[0]?.path.split('/')[0] ?? '';
  const files: SkillFilePost[] = [];
  let skipped = 0;
  for (const p of picked) {
    const rel = p.path.split('/').slice(1);
    if (!rel.length) continue;
    if (rel.some((s) => s.startsWith('.'))) { skipped++; continue; }
    files.push({ path: rel.join('/'), ...body(p.bytes) });
  }
  return { name, files, skipped };
}

/** The slug rule of names (`^[a-z0-9]…`, ≤ 41 characters); the daemon decides, this only hints. */
export const NAME_HINT = /^[a-z0-9][a-z0-9-]{0,40}$/;
