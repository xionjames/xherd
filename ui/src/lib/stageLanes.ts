import { flowOf, stageLabel } from './flow';
import type { Agent, FlowInfo } from './types';

export interface StageCol { key: string; label: string; agents: Agent[] }
/** One workflow's agents by stage. `label` is empty when it is the only lane. */
export interface Lane { key: string; label: string; cols: StageCol[] }

const NO_FLOW = 'none';

function laneKey(a: Agent): string {
  if (!a.flow) return NO_FLOW;
  return `${a.pack ?? ''}/${a.flow}`;
}

function finished(a: Agent): boolean {
  return a.status === 'done' || a.status === 'cancelled';
}

/** The Herd's By stage view (R-210, #142): a lane per workflow the given
 *  agents use, in the order of `flows` (unknown ones after, *No workflow*
 *  last), each with that workflow's own stage columns in flow order: Not
 *  started (only when used), the stages, Watching PR, Finished. An agent at a
 *  stage its flow does not list gets a column named by the stage id. Agents
 *  without a workflow have In progress, Watching PR and Finished. */
export function stageLanes(agents: Agent[], flows: FlowInfo[]): Lane[] {
  const groups = new Map<string, Agent[]>();
  for (const a of agents) {
    const k = laneKey(a);
    groups.set(k, [...(groups.get(k) ?? []), a]);
  }
  const order = [
    ...flows.map((f) => `${f.pack}/${f.flow}`),
    ...[...groups.keys()].filter((k) => k !== NO_FLOW).sort(),
    NO_FLOW,
  ];
  // An agent without a pack falls under the first flow of its name.
  for (const [k, list] of [...groups]) {
    if (k === NO_FLOW || !k.startsWith('/')) continue;
    const f = flowOf(list[0], flows);
    if (!f) continue;
    const to = `${f.pack}/${f.flow}`;
    groups.set(to, [...(groups.get(to) ?? []), ...list]);
    groups.delete(k);
  }
  const keys = [...new Set(order)].filter((k) => groups.has(k));
  const names = keys.map((k) => k.slice(k.indexOf('/') + 1));
  return keys.map((key, i) => {
    const list = groups.get(key)!;
    const cols = key === NO_FLOW ? plainCols(list) : flowCols(list, flows);
    let label = '';
    if (keys.length > 1) {
      if (key === NO_FLOW) label = 'No workflow';
      else {
        const name = names[i];
        const nice = name.charAt(0).toUpperCase() + name.slice(1);
        label = names.filter((n) => n === name).length > 1 ? `${nice} (${key.slice(0, key.indexOf('/'))})` : nice;
      }
    }
    return { key, label, cols };
  });
}

function tail(list: Agent[]): StageCol[] {
  return [
    { key: 'watching', label: 'Watching PR', agents: list.filter((a) => a.status === 'watching') },
    { key: 'finished', label: 'Finished', agents: list.filter(finished) },
  ];
}

function plainCols(list: Agent[]): StageCol[] {
  const active = list.filter((a) => a.status !== 'watching' && !finished(a));
  return [{ key: 'active', label: 'In progress', agents: active }, ...tail(list)];
}

function flowCols(list: Agent[], flows: FlowInfo[]): StageCol[] {
  const f = flowOf(list[0], flows);
  const active = list.filter((a) => a.status !== 'watching' && !finished(a));
  const cols: StageCol[] = [];
  const start = active.filter((a) => !a.stage);
  if (start.length) cols.push({ key: 'start', label: 'Not started', agents: start });
  for (const s of f?.stages ?? []) {
    cols.push({ key: `stage:${s.id}`, label: stageLabel(s), agents: active.filter((a) => a.stage === s.id) });
  }
  for (const a of active) {
    if (!a.stage || cols.some((c) => c.key === `stage:${a.stage}`)) continue;
    cols.push({ key: `stage:${a.stage}`, label: a.stage, agents: active.filter((b) => b.stage === a.stage) });
  }
  return [...cols, ...tail(list)];
}
