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
  /** Whether the commit is on the local `origin/<branch>` ref; absent when
   *  there is no such ref or the commit is not done. */
  pushed?: boolean;
}

export type Gate =
  | { kind: 'docs'; stage: string; role: Role; files: HashedFile[]; next?: string; needsHuman?: string; questions?: string[]; asked?: string[] }
  | ({ kind: 'commit' } & Commit)
  | { kind: 'pr-triage'; count: number }
  | { kind: 'pr-replies'; count: number }
  | { kind: 'question'; text: string };

/** An agent's ticket (#114): a key (`PAY-412`, `#12`), an issue URL, or both. */
export interface Ticket { key?: string; url?: string }


/** What a job keeps of the saved agent it started from (R12). */
export interface StageAgent { agent: string; human: boolean }

export interface JobAgent {
  id: string;
  name: string;
  version: number;
  instructions?: string;
  sensitivePaths?: string[];
  maxDiffLines?: number;
  jobTokenBudget?: number;
  /** What the job runs this agent on; the agent's fallback when its runtime was off (R36, R40). */
  runtime?: string;
  fellBack?: boolean;
  autonomy?: string;
  permissionMode?: string;
  model?: string;
  launch?: string[];
  /** What became of each skill the agent lists, set at creation (R26, R27). */
  skills?: SkillCopy[];
  /** The agent's memory when the job was created (R28). */
  memory?: string;
}

/** A skill copied into the job's worktree, or skipped with the reason. */
export interface SkillCopy { name: string; sha256: string; installed: boolean; skipped?: string }

/** A value a repo profile's ceiling lowered at creation or since (R12a). */
export interface Lowered { field: 'autonomy' | 'permissionMode' | string; from: string; to: string; profile: string }

/** A saved agent (R10): a named, versioned setup that jobs start from. */
export interface AgentDef {
  id: string;
  name: string;
  description: string;
  version: number;
  runtime: string;
  models?: Record<string, string>;
  /** Used once, at job creation, when `runtime` is switched off (R36). */
  fallbackRuntime?: string;
  autonomy?: 'ask' | 'important' | 'never';
  permissionMode?: import('./modes').PermissionMode;
  instructions?: string;
  jobTokenBudget?: number;
  sensitivePaths?: string[];
  maxDiffLines?: number;
  skills?: string[];
  maxLiveJobs: number;
  createdAt: number;
  updatedAt: number;
  archivedAt?: number;
}

export type AgentStatus = 'archived' | 'needs-you' | 'working' | 'idle';

/** `GET /agents` row: the settings plus what is derived from its jobs (R17, R19). */
export interface AgentView extends AgentDef {
  status: AgentStatus;
  /** Why jobs cannot start from it now (R15). */
  broken?: string;
  liveJobs: number;
  lastJob: string | null;
  tokensToday: number;
}

export interface Job {
  id: string;
  /** Lowercase slug: what the UI shows, the tmux session and worktree folder. */
  name: string;
  ticket?: Ticket;
  /** `<KEY>-<name>` with a ticket key, else the name. */
  branch: string;
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
  /** The repository has no `origin`: the ship step stopped without a pull request. */
  noOrigin?: boolean;
  tasks?: Task[];
  commits?: Commit[];
  prUrl?: string;
  /** A pending update of the repo's AGENTS.md (spec §16.6). */
  memoryProposal?: { file: string; reason: string; at: number };
  agentMemoryProposal?: { file: string; reason: string; at: number };
  /** Offer to save this job's setup as an agent (R30); never for a job with an agent. */
  suggestion?: { reason: string; at: number; dismissed: boolean };
  /** What the job proposes for that agent (R31). */
  agentDraft?: { name: string; description: string; instructions: string; at: number };
  /** What the last poll of the pull request found (R-303). */
  prState?: 'open' | 'merged' | 'closed';
  prCi?: 'pending' | 'failing' | 'passing';
  prConflict?: boolean;
  prPolledAt?: number;
  prChecks?: PrCheck[];
  prReviewers?: PrReviewer[];
  /** Merges without conflicts (from the host, or worked out in the worktree when it does not say). */
  prMergeable?: 'yes' | 'no' | 'unknown';
  prBase?: string;
  lastMessage?: string;
  /** Why the daemon blocked the agent (`git access`). */
  blockReason?: string;
  nextPrompt?: string;
  /** What xherd may approve by itself (R-109). */
  autonomy?: 'ask' | 'important' | 'never';
  /** Tool permissions (R-002a); missing on agents created before it existed. */
  permissionMode?: import('./modes').PermissionMode;
  /** Model the runtime is launched with; missing = the runtime's own default. */
  model?: string;
  /** An Ask session held open (#121): the agent stays live until Finish. */
  askHold?: { at: number; wasWatching: boolean };
  /** When the worktree folder was removed (#123); the branch stays. */
  worktreeRemovedAt?: number;
  /** The open gate is one autonomy will approve at the next stop (R4): it does not wait for you. */
  autoApprovePending?: boolean;
  /** The saved agent it started from (R12): a snapshot, so later edits do not change it. */
  agents?: Record<string, JobAgent>;
  /** The agent each stage of the workflow runs with, and whether its gate always waits for you (D26). */
  stageAgents?: Record<string, StageAgent>;
  /** Stages whose gate documents a fresh-context reader checks first, copied when the job was created. */
  freshChecks?: string[];
  /** The agent for stages with no assignment. */
  defaultAgent?: string;
  /** The agent the running session uses: a key of `agents`. */
  currentAgent?: string;
  /** An agent attached to a running job: stage assignments never switch it. */
  pinned?: boolean;
  /** Values the repo profile lowered (R12a). */
  lowered?: Lowered[];
  /** Commands the job registered or copied from the repo profile (job actions R1, R19). */
  actions?: JobAction[];
  /** The "save actions to the repo profile" card was dismissed (R23). */
  actionsCardDismissed?: boolean;
  /** First port of the job's block (`XHERD_PORT_BASE`). */
  portBase?: number;
}

/** What an action is; `cwd` is relative to the worktree, `''` is the root. */
export interface ActionDef {
  id: string;
  label: string;
  description: string;
  run: string;
  cwd: string;
  kind: 'service' | 'task';
  icon: string;
}
export type ActionState = 'running' | 'exited' | 'stopped' | 'lost';
export interface ActionRun { startedAt: number; endedAt?: number; state: ActionState; exit?: number }
/** `approved` is set when the user allowed the current command; a changed `run` or `cwd` clears it. */
export interface JobAction {
  def: ActionDef;
  source: 'job' | 'profile';
  approved?: string;
  last?: ActionRun;
  succeeded: boolean;
}
/** `GET /jobs/{id}/actions`. */
export interface ActionsView { actions: JobAction[]; portBase?: number; saveOffer: string[]; hasProfile: boolean }

/** `label`: the pack's own name for the stage ("Fix"), when it sets one. */
export interface StageInfo { id: string; role: Role; class: string; gate: boolean; label?: string }
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
  /** A runtime plugin that takes a model; its suggestions come from `GET /runtimes/{name}/models`. */
  chooseModel?: boolean;
  /** A Herd view plugin: its page, served at `/api/v1/plugins/{name}/view/{entry}`. */
  view?: { entry: string };
  /** Where an installed external plugin came from; absent for built-ins and plugins copied by hand. */
  source?: PluginSource;
}

/** The recorded source of an installed plugin (`xherd plugin install`). */
export interface PluginSource {
  url: string;
  ref?: string | null;
  path?: string | null;
  sha?: string | null;
  digest?: string | null;
  installedAt?: string | null;
  updatedAt?: string | null;
}

/** `GET /plugins`. */
export interface PluginsList {
  plugins: { name: string; kind: string; version: string; builtin: boolean; source?: PluginSource }[];
  /** The loaded plugins differ from those on disk: restart xherd. */
  restartNeeded: boolean;
  /** `false` when `XHERD_PLUGIN_INSTALL=off`: only removing works. */
  installEnabled: boolean;
}

/** `POST /plugins/preview`: what a source holds, found without running any of its code. */
export interface PluginPreview {
  url: string;
  ref?: string | null;
  /** The sub-folder with the manifest; absent at the repository root or when `choices` is set. */
  path?: string | null;
  /** The commit the ref resolved to. */
  sha: string;
  /** Several plugins in the repository: send one `path` back. */
  choices?: PluginChoice[];
  // The plugin itself (absent while `choices` asks for a path).
  name?: string;
  kind?: string;
  version?: string;
  label?: string;
  description?: string;
  minXherdVersion?: string | null;
  compatible?: boolean;
  /** Install sends this back; a different content is refused (`changed_since_preview`). */
  digest?: string;
  files?: number;
  bytes?: number;
  /** The plugin takes the place of a built-in or an installed one. */
  replaces?: 'builtin' | 'external' | null;
  settings?: { key: string; label: string; secret: boolean; required: boolean }[];
  /** Every command the plugin declares. */
  runs?: { what: string; argv: string[] }[];
  warning?: string;
}

export interface PluginChoice { path: string; name: string; kind?: string | null; error?: string }

/** `POST /plugins/{name}/update` without a digest. */
export interface PluginUpdatePreview extends PluginPreview {
  installed: { url: string; ref?: string | null; path?: string | null; sha?: string | null; version?: string | null; digest?: string | null };
  /** Unified diff of the installed manifest against the new one; empty when equal. */
  manifestDiff: string;
}

/** `POST /plugins/install` and `POST /plugins/{name}/update` with a digest. */
export interface InstalledPlugin {
  name: string;
  kind?: string;
  version: string;
  url: string;
  ref?: string | null;
  path?: string | null;
  sha: string;
  digest: string;
  /** Update only. */
  previousSha?: string | null;
}

export interface ModelSuggestion { id: string; label?: string; tier?: string }

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
  jobs: Job[];
  agents: AgentView[];
  plugins: PluginInfo[];
  /** Link rules of the enabled tracker plugins. */
  trackerLinks: { plugin: string; pattern: string; url: string }[];
  brokenPlugins: { name: string; dir: string; error: string }[];
  flows: FlowInfo[];
  /** Running in the xherd container. */
  sandbox: boolean;
}

export interface DiffAnswer {
  job: string;
  head: string;
  commit?: Commit;
  view?: 'uncommitted' | 'branch' | 'commit';
  branch?: string;
  files: string[];
  others: string[];
  diff: string;
}

/** `GET /jobs/:id/diff/source`: the new side of one file of a diff. */
export interface DiffSource {
  path: string;
  rev: string;
  text: string;
}

export interface Check {
  id: string;
  group: 'system' | 'plugins' | 'runtime' | 'security';
  label: string;
  required: boolean;
  ok: boolean;
  detail: string;
  /** The plugin's CLI is being installed or updated: no fix is offered meanwhile. */
  installing?: boolean;
  fix?:
    | { kind: 'command'; text: string }
    | { kind: 'action'; label: string }
    | { kind: 'review'; label: string }
    | { kind: 'session'; label: string; explain: string; terminal: string };
}

/** A pack file waiting for a decision (`GET /install/review`): edited or obsolete. */
export interface PackFile {
  runtime: string;
  path: string;
  state: 'edited' | 'obsolete';
  /** Obsolete only: on disk and different from what xherd recorded. */
  edited: boolean;
}

export type PackChoiceValue = 'use-new' | 'keep-mine' | 'remove' | 'keep';

/** One decision of `POST /install/resolve`. */
export interface PackChoice {
  runtime: string;
  path: string;
  choice: PackChoiceValue;
}

export interface ConfigView {
  config: Record<string, unknown>;
  running: Record<string, unknown>;
  restartNeeded: boolean;
}

/** `POST /jobs/{id}/files`: the absolute paths the files were stored under. */
export interface UploadResult {
  paths: string[];
}

export type CheckFix = NonNullable<Check['fix']>;

export interface GitHost {
  host: string;
  protocols: ('https' | 'ssh')[];
  credential: 'none' | 'provider' | 'xherd';
  /** The git provider plugin; absent = the one claiming the host name. */
  provider?: string;
  username?: string;
  caFile?: string;
  preferHttps: boolean;
  preferSsh: boolean;
  /** A token is stored in secrets.json (never sent). */
  hasToken?: boolean;
  /** The plugin this implicit row comes from; never saved. */
  from?: string;
}

export interface GitProvider {
  name: string;
  /** Host name patterns it claims (`*` = any run of characters). */
  hosts: string[];
  /** It supplies a git credential helper / a browser login. */
  helper: boolean;
  login: boolean;
}

export interface GitHostsView {
  hosts: GitHost[];
  providers: GitProvider[];
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

/** Settings → Open in, as saved in `config.json` (schema 3). */
export interface OpenInConfig {
  hidden?: string[];
  terminal?: string;
  links?: { id: string; label: string; url: string }[];
  droppedCommands?: string[];
}

/** What the terminal handler's installer reported (`<data>/terminal-handler.json`). */
export interface HandlerRecord {
  version: number;
  xherd: string;
  os: string;
  terminals: string[];
  root: string;
  at: number;
}

/** `GET /open-in`. */
export interface OpenInView {
  links: { id: string; label: string }[];
  native: boolean;
  handler: { shipped: number; installed: HandlerRecord | null; updateAvailable: boolean; rootChanged: boolean };
}

/** `GET /skills` row (R25). */
export interface SkillInfo {
  name: string;
  description: string;
  files: number;
  bytes: number;
  sha256: string;
  /** Names of the agents that list it. */
  usedBy: string[];
}

/** `GET /skills/{name}`. */
export interface SkillDetail extends SkillInfo {
  fileList: { path: string; bytes: number }[];
  /** The text of SKILL.md. */
  text: string;
}

/** One stage of a workflow as `GET /flows` shows it (D26, R35): the agent it
 *  resolves to (its assignment, else the stage before's) and why it cannot run. */
export interface FlowStageView {
  id: string;
  label?: string;
  role: Role;
  class: string;
  command?: string;
  /** The stage has an assignment row of its own. */
  assigned: boolean;
  human: boolean;
  agentId?: string;
  agent?: string;
  broken?: string;
  /** A fresh-context reader checks this stage's documents before its gate. */
  freshCheck: boolean;
  /** The stage may have the check (a spec or design stage with a documents gate). */
  freshCheckAllowed: boolean;
}
export interface FlowView { pack: string; flow: string; stages: FlowStageView[] }
/** A row of `PUT /flows/{pack}/{flow}/assignments`. */
export interface AssignmentRow { stage: string; agentId: string; human: boolean }
