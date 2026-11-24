// Pure helpers for images: which paths are images, how a Markdown `src` is
// read (R1-R4), and what the Changes panel shows for a file (R7-R8).
import type { FileDiff } from './diff';

export const IMAGE_EXT = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg'];

const ext = (p: string) => (p.includes('.') ? p.split('.').pop()!.toLowerCase() : '');
export const isImage = (p: string) => IMAGE_EXT.includes(ext(p));
export const isSvg = (p: string) => ext(p) === 'svg';

export type Reason = 'outside the worktree' | 'not found' | 'not an image' | 'too large';
export type ImgRef =
  | { kind: 'local'; path: string }
  | { kind: 'data' }
  | { kind: 'remote'; url: string }
  | { kind: 'bad'; reason: Reason };

const DATA_IMAGE = /^data:image\/(png|jpeg|gif|webp|svg\+xml)[;,]/i;

/** How the `src` of an image in a document at directory `base` (relative to the worktree root, '' for the root) is read. */
export function resolveImage(src: string, base: string): ImgRef {
  const s = src.trim();
  if (/^data:/i.test(s)) return DATA_IMAGE.test(s) ? { kind: 'data' } : { kind: 'bad', reason: 'not an image' };
  if (s.startsWith('//')) return { kind: 'remote', url: `https:${s}` };
  if (/^[a-z][a-z0-9+.-]*:/i.test(s)) return { kind: 'remote', url: s };

  let p = s.replace(/[?#].*$/s, '');
  try { p = decodeURIComponent(p); } catch { /* keep as written */ }
  if (p.split('/').every((seg) => seg === '' || seg === '.')) return { kind: 'bad', reason: 'not found' };
  const rooted = p.startsWith('/');
  const parts = [...(rooted ? [] : base.split('/')), ...p.split('/')];
  const out: string[] = [];
  for (const seg of parts) {
    if (seg === '' || seg === '.') continue;
    if (seg === '..') {
      if (!out.length) return { kind: 'bad', reason: 'outside the worktree' };
      out.pop();
    } else out.push(seg);
  }
  const rel = out.join('/');
  if (!rel) return { kind: 'bad', reason: 'not found' };
  if (!isImage(rel)) return { kind: 'bad', reason: 'not an image' };
  return { kind: 'local', path: `./${rel}` };
}

/** Directory of a document of the Documents panel: `./a/b.md` is in the worktree, other paths are in `.xherd/spec`. */
export function docBase(docPath: string): string {
  const dir = (p: string) => p.split('/').slice(0, -1).join('/');
  if (docPath.startsWith('./')) return dir(docPath.slice(2));
  const d = dir(docPath);
  return d ? `.xherd/spec/${d}` : '.xherd/spec';
}

/** Directory of a file of the Changes panel (a worktree path). */
export const fileBase = (path: string) => path.split('/').slice(0, -1).join('/');

/** Why an image could not be read, from the daemon's error. */
export function reasonOf(err: unknown): string {
  const code = (err as { code?: string } | null)?.code;
  switch (code) {
    case 'bad_path': return 'outside the worktree';
    case 'no_such_doc':
    case 'doc_missing': return 'not found';
    case 'not_an_image': return 'not an image';
    case 'too_large': return 'too large';
    default: return err instanceof Error ? err.message : String(err);
  }
}

export const previewKind = (p: string): 'markdown' | 'html' | 'image' | null => {
  const e = ext(p);
  return e === 'md' || e === 'markdown' ? 'markdown' : e === 'html' || e === 'htm' ? 'html' : isImage(p) ? 'image' : null;
};

/** Whether a file section offers Preview. */
export function hasPreview(f: FileDiff): boolean {
  const k = previewKind(f.path);
  if (k === 'image') return true;
  return !!k && f.status !== 'deleted' && !f.note;
}

export interface ImageSides {
  before?: { path: string };
  after?: { path: string } | 'too large';
}

/** The images a file section shows: Before (old side) and After (new side). */
export function imageSides(f: FileDiff): ImageSides {
  if (f.note) return { before: { path: f.oldPath ?? f.path }, after: 'too large' };
  switch (f.status) {
    case 'added': return { after: { path: f.path } };
    case 'deleted': return { before: { path: f.path } };
    case 'renamed': return { before: { path: f.oldPath ?? f.path }, after: { path: f.path } };
    default: return { before: { path: f.path }, after: { path: f.path } };
  }
}

/** A raster image has no text diff, so its section opens on Preview. By extension, not by the `binary` flag
 *  (a pure rename or a mode change has no `Binary files` line). */
export const opensOnPreview = (f: FileDiff) => isImage(f.path) && !isSvg(f.path);

/** The diff's (abbreviated) blob id is not the start of the one served: the file changed since. */
export function stale(newBlob: string | undefined | null, served: string | undefined | null): boolean {
  if (!newBlob || !served || /^0+$/.test(newBlob) || /^0+$/.test(served)) return false;
  return !served.startsWith(newBlob);
}
