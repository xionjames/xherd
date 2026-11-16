// Herd view "By agent" (D26, R43): one row per saved agent with the jobs whose
// session runs it now, agents with jobs first, `No agent` last. Pure, tested.
import type { AgentView, Job } from './types';
import { NO_AGENT_LANE } from './agents';

export interface AgentRowData {
  key: string;
  label: string;
  agent?: AgentView;
  jobs: Job[];
}

export interface RowOptions {
  /** `?agent=NAME`: only that agent's row. */
  agent?: string | null;
  /** `?archived=1`: archived agents get a row too. */
  archived?: boolean;
}

/** Rows for the jobs shown: each agent in name order, those with jobs first;
 *  the `No agent` row only when a job has none. Without `archived`, an
 *  archived agent has no row (its jobs are not listed here). */
export function agentRows(jobs: readonly Job[], agents: readonly AgentView[], o: RowOptions = {}): AgentRowData[] {
  const rows: AgentRowData[] = agents
    .filter((a) => o.archived || a.status !== 'archived')
    .map((a) => ({ key: a.id, label: a.name, agent: a, jobs: jobs.filter((j) => j.currentAgent === a.id) }));
  rows.sort((a, b) => Number(b.jobs.length > 0) - Number(a.jobs.length > 0) || a.label.localeCompare(b.label));
  const plain = jobs.filter((j) => !j.currentAgent || !j.agents?.[j.currentAgent]);
  if (plain.length) rows.push({ key: '', label: NO_AGENT_LANE, jobs: plain });
  return o.agent ? rows.filter((r) => r.label === o.agent) : rows;
}
