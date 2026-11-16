// The New agent form's Repository field when the ticket names it (#148).
// `typed` is what the field holds now, `filled` the value this helper put
// there last (null when it never did), `ticketRepo` what the daemon derived
// from the ticket now (null when it gives none).
export interface RepoState {
  repo: string;
  filled: string | null;
}

export function nextRepo(s: { typed: string; filled: string | null; ticketRepo: string | null }): RepoState {
  const ours = s.typed.trim() === '' || (s.filled !== null && s.typed === s.filled);
  // The user's own text always wins; it also stops being "ours".
  if (!ours) return { repo: s.typed, filled: null };
  // An empty or self-filled field follows the ticket: filled, replaced, or
  // emptied again when the ticket no longer gives a repository.
  if (s.ticketRepo) return { repo: s.ticketRepo, filled: s.ticketRepo };
  return { repo: '', filled: null };
}
