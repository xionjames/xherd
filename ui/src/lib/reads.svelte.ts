// What the user has read, per agent: document → hash. Approving sends these
// so the daemon refuses if a document changed after it was read (R-106).
export const reads = $state<Record<string, Record<string, string>>>({});

export function markRead(agentId: string, path: string, sha256: string) {
  (reads[agentId] ??= {})[path] = sha256;
}
