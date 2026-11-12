// Wire types: the daemon's JSON (camelCase fields, kebab-case enum values).

export type Status =
  | 'queued' | 'running' | 'waiting-input' | 'blocked' | 'idle' | 'parked'
  | 'paused' | 'watching' | 'done' | 'cancelled' | 'error';

export type Role =
  | 'explore' | 'spec' | 'design' | 'tasks' | 'implement' | 'review' | 'ship' | 'pr' | 'custom';

export interface HashedFile { path: string; sha256: string }

export interface Task {
  id: string;
  title: string;
  status: 'todo' | 'doing' | 'done' | 'blocked';
  depends: string[];
  files: string[];
  evidence?: string;
  verify?: string;
  blockedReason?: string;
  /** Where the task came from; absent = the approved task list. */
  source?: 'review' | 'pr';
}

/** A comment on the pull request (R-304); `body` is untrusted text. */
/** One check run of the pull request; `link` is host text (show only https). */
export interface PrCheck {
  name: string;
  bucket: 'pass' | 'fail' | 'pending' | 'skipping' | 'cancel' | string;
  workflow?: string;
  link?: string;
}

export interface PrReviewer {
  name: string;
  state: 'pending' | 'approved' | 'changes-requested' | 'commented' | 'dismissed';
}

export interface PrComment {
  id: string;
  kind: string;
  author: string;
  body: string;
  url?: string;
  createdAt?: string;
  path?: string;
  line?: number;
  inReplyTo?: string;
}

export type Decision = 'fix' | 'reply' | 'skip';

/** What to do about a comment: the agent's proposal or the user's decision (R-306). */
export interface Verdict {
  id: string;
  decision: Decision;
  note?: string;
  reply?: string;
}

export type Severity = 'minor' | 'major' | 'blocker';

/** A finding of `review.md`; `resolved`: the tasks that fix it are done. */
export interface Finding {
  id: string;
  title: string;
  severity: Severity;
  location?: string;
  fixed?: string[];
  left?: string;
  resolved: boolean;
}

export interface Commit {
  id: string;
  subject: string;
  /** The full commit message (as proposed, or as the user edited it). */
  message?: string;
  files: string[];
  status: 'proposed' | 'approved' | 'done' | 'dropped';
  tasks: string[];
  sha?: string;
  needsHuman?: string;
}

export type Gate =
  | { kind: 'docs'; stage: string; role: Role; files: HashedFile[]; next?: string; needsHuman?: string; questions?: string[]; asked?: string[] }
  | ({ kind: 'commit' } & Commit)
  | { kind: 'pr-triage'; count: number }
  | { kind: 'pr-replies'; count: number }
  | { kind: 'question'; text: string };

export interface Agent {
  id: string;
  name: string;
  ticketKey: string;
  status: Status;
  createdAt: number;
  statusSince: number;
  lastActivity: number;
  /** A pause is waiting for the agent's handoff note. */
  pauseRequestedAt?: number;
  workdir?: string;
  repo?: string;
  base?: string;
  runtime?: string;
  pack?: string;
  flow?: string;
  stage?: string;
  /** Stages entered, in order; earlier ones missing were skipped. */
  visitedStages?: string[];
  gate?: Gate;
  tasks?: Task[];
  commits?: Commit[];
  prUrl?: string;
  /** A pending update of the repo's AGENTS.md (spec §16.6). */
  memoryProposal?: { file: string; reason: string; at: number };
  /** What the last poll of the pull request found (R-303). */
  prState?: 'open' | 'merged' | 'closed';
  prCi?: 'pending' | 'failing' | 'passing';
  prConflict?: boolean;
  prPolledAt?: number;
  prChecks?: PrCheck[];
  prReviewers?: PrReviewer[];
  /** Merges without conflicts (from the host, or worked out in the worktree when it does not say). */
  prMergeable?: 'yes' | 'no' | 'unknown';
  lastMessage?: string;
  /** Why the daemon blocked the agent (`git access`). */
  blockReason?: string;
  nextPrompt?: string;
  /** What xherd may approve by itself (R-109). */
  autonomy?: 'ask' | 'important' | 'never';
  /** Tool permissions (R-002a); missing on agents created before it existed. */
  permissionMode?: import('./modes').PermissionMode;
}

export interface StageInfo { id: string; role: Role; class: string; gate: boolean }
export interface FlowInfo { pack: string; flow: string; stages: StageInfo[] }

export interface PluginInfo {
  name: string;
  kind: string;
  version: string;
  label?: string;
  builtin: boolean;
  description?: string;
  settings: PluginSetting[];
  /** Switched on (read live from the saved config). */
  enabled: boolean;
  /** Has an interactive login; its status is the check `plugin:<name>:login`. */
  login: boolean;
}

export interface PluginSetting {
  key: string;
  label: string;
  type: string;
  default?: unknown;
  secret: boolean;
  required: boolean;
}

export interface AgentEvent { id: number; kind: string; payload: unknown; at: number }

export interface AppState {
  version: string;
  /** Release tag, or `<tag>+<commits>.<hash>` in development. */
  buildVersion: string;
  platform: string;
  config: Record<string, unknown>;
  agents: Agent[];
  plugins: PluginInfo[];
  brokenPlugins: { name: string; dir: string; error: string }[];
  flows: FlowInfo[];
  /** Running in the xherd container. */
  sandbox: boolean;
}

export interface DiffAnswer {
  agent: string;
  head: string;
  commit?: Commit;
  view?: 'uncommitted' | 'branch' | 'commit';
  branch?: string;
  files: string[];
  others: string[];
  diff: string;
}

export interface Check {
  id: string;
  group: 'system' | 'plugins' | 'runtime' | 'security';
  label: string;
  required: boolean;
  ok: boolean;
  detail: string;
  fix?:
    | { kind: 'command'; text: string }
    | { kind: 'action'; label: string }
    | { kind: 'session'; label: string; explain: string };
}

export interface ConfigView {
  config: Record<string, unknown>;
  running: Record<string, unknown>;
  restartNeeded: boolean;
}

export type CheckFix = NonNullable<Check['fix']>;

export interface GitHost {
  host: string;
  protocols: ('https' | 'ssh')[];
  credential: 'none' | 'gh' | 'glab' | 'xherd';
  username?: string;
  caFile?: string;
  preferHttps: boolean;
  preferSsh: boolean;
  /** A token is stored in secrets.json (never sent). */
  hasToken?: boolean;
}

export interface GitHostsView {
  hosts: GitHost[];
  sandbox: boolean;
  gitconfig: string | null;
}

export interface GitFinding {
  id: string;
  label: string;
  ok: boolean;
  detail: string;
  fix?: CheckFix;
}
