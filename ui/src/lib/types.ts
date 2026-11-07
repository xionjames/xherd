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
}

export interface Commit {
  id: string;
  subject: string;
  files: string[];
  status: 'proposed' | 'approved' | 'done' | 'dropped';
  tasks: string[];
  sha?: string;
  needsHuman?: string;
}

export type Gate =
  | { kind: 'docs'; stage: string; role: Role; files: HashedFile[]; next?: string; needsHuman?: string; questions?: string[] }
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
  lastMessage?: string;
  nextPrompt?: string;
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
