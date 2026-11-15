import { describe, expect, it } from 'vitest';
import { stageLanes } from './stageLanes';
import type { Job, FlowInfo } from './types';

const st = (id: string, role: string, label?: string) => ({ id, role: role as never, class: 'routine', gate: true, label });
const feature: FlowInfo = { pack: 'xherd', flow: 'feature', stages: [st('spec', 'spec'), st('tasks', 'tasks'), st('implement', 'implement')] };
const bugfix: FlowInfo = {
  pack: 'xherd', flow: 'bugfix',
  stages: [st('analysis', 'spec'), st('reproduce', 'implement'), st('implement', 'implement', 'Fix')],
};
const flows = [feature, bugfix];
let n = 0;
const a = (over: Partial<Job>): Job => {
  n += 1;
  return { id: `a${n}`, name: `a${n}`, branch: 'b', status: 'running', createdAt: 0, statusSince: 0, lastActivity: 0, ...over };
};
const shape = (lanes: ReturnType<typeof stageLanes>) =>
  lanes.map((l) => ({ label: l.label, cols: l.cols.map((c) => `${c.label}:${c.agents.length}`) }));

describe('stageLanes', () => {
  it('one workflow gives one unlabelled lane with its stage columns', () => {
    const lanes = stageLanes([a({ pack: 'xherd', flow: 'bugfix', stage: 'reproduce' })], flows);
    expect(shape(lanes)).toEqual([
      { label: '', cols: ['Analysis:0', 'Reproduce:1', 'Fix:0', 'Watching PR:0', 'Finished:0'] },
    ]);
  });

  it('feature, bugfix and no workflow give three lanes with their own columns and counts', () => {
    const lanes = stageLanes([
      a({ pack: 'xherd', flow: 'bugfix', stage: 'analysis' }),
      a({ pack: 'xherd', flow: 'feature', stage: 'spec' }),
      a({ pack: 'xherd', flow: 'feature', stage: 'spec', status: 'watching' }),
      a({}),
      a({ status: 'done' }),
    ], flows);
    expect(shape(lanes)).toEqual([
      { label: 'Feature', cols: ['Spec:1', 'Tasks:0', 'Build:0', 'Watching PR:1', 'Finished:0'] },
      { label: 'Bugfix', cols: ['Analysis:1', 'Reproduce:0', 'Fix:0', 'Watching PR:0', 'Finished:0'] },
      { label: 'No workflow', cols: ['In progress:1', 'Watching PR:0', 'Finished:1'] },
    ]);
  });

  it('a workflow without shown agents has no lane', () => {
    expect(stageLanes([a({ pack: 'xherd', flow: 'feature', stage: 'spec' })], flows).map((l) => l.key)).toEqual(['xherd/feature']);
    expect(stageLanes([], flows)).toEqual([]);
  });

  it('Not started only when an agent has no stage yet', () => {
    const lanes = stageLanes([a({ pack: 'xherd', flow: 'feature', status: 'queued' })], flows);
    expect(lanes[0].cols[0].label).toBe('Not started');
    expect(lanes[0].cols[0].agents).toHaveLength(1);
  });

  it('a stage the flow does not list gets its own column; an agent without a pack joins its flow', () => {
    const lanes = stageLanes([
      a({ pack: 'xherd', flow: 'feature', stage: 'legacy' }),
      a({ flow: 'feature', stage: 'tasks' }),
    ], flows);
    expect(shape(lanes)).toEqual([
      { label: '', cols: ['Spec:0', 'Tasks:1', 'Build:0', 'legacy:1', 'Watching PR:0', 'Finished:0'] },
    ]);
  });
});
