import type { Agent, FlowInfo, StageInfo } from './types';

export type StepState = 'ok' | 'now' | 'cur' | 'todo';

export const STAGE_LABEL: Record<string, string> = {
  explore: 'Explore', spec: 'Spec', design: 'Design', tasks: 'Tasks', implement: 'Build',
  review: 'Review', ship: 'Ship', pr: 'PR',
};

export function flowOf(a: Agent, flows: FlowInfo[]): FlowInfo | undefined {
  if (!a.flow) return undefined;
  return flows.find((f) => f.flow === a.flow && (!a.pack || f.pack === a.pack));
}

export function stageLabel(s: StageInfo): string {
  return STAGE_LABEL[s.role] ?? s.id;
}

/** State of each stage: done before the current one, waiting (gate open) or
 *  running at the current one, nothing after. */
export function steps(a: Agent, f: FlowInfo): { stage: StageInfo; state: StepState }[] {
  const cur = a.stage ? f.stages.findIndex((s) => s.id === a.stage) : -1;
  const finished = a.status === 'done' || a.status === 'watching';
  return f.stages.map((stage, i) => {
    let state: StepState = 'todo';
    if (finished && (cur < 0 || i <= cur)) state = 'ok';
    else if (i < cur) state = 'ok';
    else if (i === cur) state = a.gate ? 'now' : 'cur';
    return { stage, state };
  });
}
