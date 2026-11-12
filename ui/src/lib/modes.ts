// Tool-permission modes (R-002a): what the agent's runtime may do without
// asking. `all` asks nothing, so the daemon offers it only in the container.
export type PermissionMode = 'ask' | 'edits' | 'auto' | 'all';

export const MODE_HINT: Record<PermissionMode, string> = {
  ask: 'Asks before every tool use that needs permission.',
  edits: 'Accepts file edits; commands that change things still ask (the agent shows as blocked).',
  auto: "The runtime's own safety checks decide what to ask.",
  all: 'Asks nothing. Only in the xherd container.',
};

export const modes = (sandbox: boolean): PermissionMode[] =>
  (['ask', 'edits', 'auto', 'all'] as PermissionMode[]).filter((m) => sandbox || m !== 'all');
