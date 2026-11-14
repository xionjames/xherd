// Herd view plugins, page side of protocol v1 (pure; R9, R13).
//
// A view plugin runs in a sandboxed iframe with an opaque origin. The Herd
// page sends it `data` messages holding a projection of the agents (never
// their prompts, messages, question texts, documents, comments or paths)
// and accepts only `ready` and `open` back. Keep the shapes in step with
// `ui/view-kit/xherd-view.js`.
import { stageNow } from './flow';
import { agentHue, needsYou, repoName } from './format';
import type { Agent, FlowInfo, Gate, Status, Ticket } from './types';

export const VIEW_PROTOCOL = 1;

/** What a view plugin sees of an agent. */
export interface ViewAgent {
  id: string;
  name: string;
  status: Status;
  statusSince: number;
  createdAt: number;
  lastActivity: number;
  /** The repository's folder name. */
  repo: string;
  branch: string;
  ticket?: Ticket;
  runtime?: string;
  flow?: string;
  stage?: string;
  /** The stage's label (its role's name). */
  stageName?: string;
  /** What the open gate waits on: document names, a commit id, a count. */
  waitsOn?: string;
  /** The open gate's kind only. */
  gate?: Gate['kind'];
  needsYou: boolean;
  /** The agent's colour, a CSS `hsl(…)`. */
  hue: string;
  prUrl?: string;
  prState?: Agent['prState'];
  prCi?: Agent['prCi'];
}

export interface DataMessage {
  xherdView: typeof VIEW_PROTOCOL;
  kind: 'data';
  agents: ViewAgent[];
  flows: FlowInfo[];
  theme: 'light' | 'dark';
}

export type ViewRequest = { kind: 'ready' } | { kind: 'open'; id: string };

const str = (v: unknown) => (typeof v === 'string' ? v : undefined);

/** The projection of one agent; fields not listed here never reach a view. */
export function project(a: Agent, flows: FlowInfo[]): ViewAgent {
  const now = stageNow(a, flows);
  const ticket = a.ticket ? { key: str(a.ticket.key), url: str(a.ticket.url) } : undefined;
  return strip({
    id: a.id,
    name: a.name,
    status: a.status,
    statusSince: a.statusSince,
    createdAt: a.createdAt,
    lastActivity: a.lastActivity,
    repo: repoName(a),
    branch: a.branch,
    ticket: ticket && strip(ticket),
    runtime: a.runtime,
    flow: a.flow,
    stage: a.stage,
    stageName: now?.name,
    waitsOn: now?.waits,
    gate: a.gate?.kind,
    needsYou: needsYou(a),
    hue: agentHue(a.name),
    prUrl: a.prUrl,
    prState: a.prState,
    prCi: a.prCi,
  });
}

/** Drops undefined fields, so the message holds only what is known. */
function strip<T extends object>(o: T): T {
  return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined)) as T;
}

export function dataMessage(agents: Agent[], flows: FlowInfo[], theme: 'light' | 'dark'): DataMessage {
  return {
    xherdView: VIEW_PROTOCOL,
    kind: 'data',
    agents: agents.map((a) => project(a, flows)),
    flows: flows.map((f) => ({ pack: f.pack, flow: f.flow, stages: f.stages.map((s) => ({ id: s.id, role: s.role, class: s.class, gate: s.gate })) })),
    theme,
  };
}

/**
 * A message from the view, or null to ignore it: it must come from the
 * frame's window, with the opaque origin `'null'`, protocol v1, and be
 * `ready` or `open` of an agent the page sent.
 */
export function acceptMessage(
  e: { source: unknown; origin: string; data: unknown },
  frame: unknown,
  ids: ReadonlySet<string>,
): ViewRequest | null {
  if (!frame || e.source !== frame || e.origin !== 'null') return null;
  const m = e.data;
  if (!m || typeof m !== 'object') return null;
  const d = m as Record<string, unknown>;
  if (d.xherdView !== VIEW_PROTOCOL) return null;
  if (d.kind === 'ready') return { kind: 'ready' };
  if (d.kind === 'open' && typeof d.agentId === 'string' && ids.has(d.agentId)) return { kind: 'open', id: d.agentId };
  return null;
}
