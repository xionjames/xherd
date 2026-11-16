import type { Job } from './types';
import { repoName } from './format';
import { NO_AGENT_LANE, agentLane } from './agents';

/** The Board's swimlanes (`?lanes=`). */
export type Lanes = 'none' | 'agent' | 'agent-def' | 'repo' | 'flow';
export const LANES: Lanes[] = ['none', 'agent', 'agent-def', 'repo', 'flow'];

export const TODO_LANE = 'TODOs';
export const NO_FLOW_LANE = 'No workflow';

/** The lane a job's cards go in: none, its name, the saved agent it started
 *  from (`agent-def`; `No agent` without one), its repository, or its
 *  workflow (`Feature`, `Bugfix`; `No workflow` for flow `none`). */
export function boardLane(a: Job, lanes: Lanes): string {
  if (lanes === 'agent') return a.name;
  if (lanes === 'agent-def') return agentLane(a);
  if (lanes === 'repo') return repoName(a);
  if (lanes === 'flow') return a.flow ? a.flow.charAt(0).toUpperCase() + a.flow.slice(1) : NO_FLOW_LANE;
  return '';
}

/** TODOs belong to no agent: their own lane whenever there are lanes. */
export function todoLane(lanes: Lanes): string {
  return lanes === 'none' ? '' : TODO_LANE;
}

/** Lanes in name order, `No workflow` and `No agent` then `TODOs` last. */
export function laneOrder(names: string[]): string[] {
  const rank = (n: string) => (n === TODO_LANE ? 2 : n === NO_FLOW_LANE || n === NO_AGENT_LANE ? 1 : 0);
  return [...new Set(names)].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
}
