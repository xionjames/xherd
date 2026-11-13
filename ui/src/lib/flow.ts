import type { Agent, FlowInfo, StageInfo } from './types';

export type StepState = 'ok' | 'skip' | 'now' | 'cur' | 'todo';

export const STAGE_LABEL: Record<string, string> = {
  explore: 'Explore', spec: 'Spec', design: 'Design', tasks: 'Tasks', implement: 'Build',
  review: 'Review', ship: 'Ship', pr: 'PR',
};

export function flowOf(a: Agent, flows: FlowInfo[]): FlowInfo | undefined {
  if (!a.flow) return undefined;
  return flows.find((f) => f.flow === a.flow && (!a.pack || f.pack === a.pack));
}

/** "Spec", "Build"…; a stage whose id is not its role names itself by its
 *  id, known words as their labels (`pr-review` → "PR Review"), so two stages
 *  with one role (pr-review, pr) read differently. */
export function stageLabel(s: StageInfo): string {
  if (s.id === s.role) return STAGE_LABEL[s.role] ?? s.id;
  return s.id.split(/[-_]/).map((w) => STAGE_LABEL[w] ?? w.charAt(0).toUpperCase() + w.slice(1)).join(' ');
}

/** State of each stage: done (or skipped, when the agent never entered it)
 *  before the current one, waiting (gate open) or running at the current one,
 *  nothing after. A done agent has every stage behind it; a watching one is
 *  at the flow's last PR stage (watching the PR is that stage's work), with
 *  the stages it went through done. Agents without a record of visited
 *  stages show all done. */
export function steps(a: Agent, f: FlowInfo): { stage: StageInfo; state: StepState }[] {
  const cur = a.stage ? f.stages.findIndex((s) => s.id === a.stage) : -1;
  const visited = a.visitedStages ?? [];
  const passed = (id: string): StepState => (visited.length && !visited.includes(id) ? 'skip' : 'ok');
  const watchAt = a.status === 'watching' ? f.stages.map((s) => s.role).lastIndexOf('pr') : -1;
  const done = a.status === 'done' || (a.status === 'watching' && watchAt < 0);
  return f.stages.map((stage, i) => {
    let state: StepState = 'todo';
    if (done) state = i === cur ? 'ok' : passed(stage.id);
    else if (watchAt >= 0) {
      if (i === watchAt) state = 'cur';
      else if (i <= cur) state = passed(stage.id);
      else if (!visited.length || visited.includes(stage.id)) state = 'ok';
    } else if (i < cur) state = passed(stage.id);
    else if (i === cur) state = a.gate ? 'now' : 'cur';
    return { stage, state };
  });
}

/** The agent's current stage as the Herd shows it: its name ("Ship") and,
 *  while its gate is open, what waits there ("pr.md", "C05"). */
export function stageNow(a: Agent, flows: FlowInfo[]): { name: string; waits?: string } | undefined {
  if (!a.stage) return undefined;
  const s = flowOf(a, flows)?.stages.find((x) => x.id === a.stage);
  const name = s ? stageLabel(s) : a.stage;
  const g = a.gate;
  const waits = !g ? undefined
    : g.kind === 'docs' ? g.files.map((f) => f.path.split('/').pop()).join(', ') || undefined
    : g.kind === 'commit' ? g.id
    : g.kind === 'pr-triage' ? `${g.count} comments`
    : g.kind === 'pr-replies' ? `${g.count} replies`
    : 'a question';
  return { name, waits };
}
