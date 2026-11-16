import { describe, expect, it } from 'vitest';
import { agentRows } from './herdAgents';
import { assignmentRows, mixedAgents, resolveStages, stageRuns } from './agents';
import type { AgentView, FlowStageView, Job } from './types';

const ag = (id: string, name: string, status: AgentView['status'] = 'idle'): AgentView => ({ id, name, status } as AgentView);
const job = (id: string, current?: string): Job =>
  ({ id, name: id, currentAgent: current, agents: current ? { [current]: { id: current, name: current } } : {} } as unknown as Job);

describe('agentRows', () => {
  const agents = [ag('b', 'bravo'), ag('a', 'alpha'), ag('c', 'charlie'), ag('z', 'old', 'archived')];
  it('lists agents with jobs first, by name, and No agent last', () => {
    const rows = agentRows([job('j1', 'b'), job('j2'), job('j3', 'b')], agents);
    expect(rows.map((r) => r.label)).toEqual(['bravo', 'alpha', 'charlie', 'No agent']);
    expect(rows[0].jobs.map((j) => j.id)).toEqual(['j1', 'j3']);
  });
  it('has no No agent row when every job has an agent', () => {
    expect(agentRows([job('j1', 'a')], agents).map((r) => r.label)).not.toContain('No agent');
  });
  it('hides archived agents unless asked', () => {
    expect(agentRows([], agents).map((r) => r.label)).not.toContain('old');
    expect(agentRows([], agents, { archived: true }).map((r) => r.label)).toContain('old');
  });
  it('filters by agent name', () => {
    expect(agentRows([job('j1', 'a')], agents, { agent: 'alpha' }).map((r) => r.label)).toEqual(['alpha']);
  });
});

const st = (id: string, over: Partial<FlowStageView> = {}): FlowStageView => ({ id, role: 'spec', class: 'human-only', assigned: false, human: false, ...over } as FlowStageView);

describe('resolveStages', () => {
  const stages = [st('spec', { assigned: true, agent: 'writer' }), st('design'), st('tasks', { assigned: true, agent: 'planner', human: true })];
  it('takes the assignment, then the job agent, then the previous stage', () => {
    expect(resolveStages(stages, undefined, {}).map((p) => [p.agent, p.from])).toEqual([['writer', 'assignment'], ['writer', 'previous'], ['planner', 'assignment']]);
    expect(resolveStages(stages, 'main', {}).map((p) => p.agent)).toEqual(['writer', 'main', 'planner']);
  });
  it('lets the request override a stage', () => {
    const r = resolveStages(stages, undefined, { design: 'other' });
    expect(r[1]).toMatchObject({ agent: 'other', from: 'override' });
  });
  it('gives no agent to a flow with none', () => {
    expect(resolveStages([st('spec')], undefined, {})[0]).toMatchObject({ agent: undefined, from: 'none' });
  });
});

describe('stageRuns', () => {
  const j = { stage: 'design', currentAgent: 'a', agents: { a: { name: 'alpha' }, b: { name: 'bravo' } }, stageAgents: { spec: { agent: 'a', human: false }, design: { agent: 'b', human: true } } } as unknown as Job;
  it('names the agent of each stage from the snapshot', () => {
    const runs = stageRuns(j, [{ id: 'spec' }, { id: 'design' }, { id: 'tasks' }]);
    expect(runs.map((r) => r.agent)).toEqual(['alpha', 'bravo', undefined]);
    expect(runs[1]).toMatchObject({ human: true, current: true });
    expect(mixedAgents(runs)).toBe(true);
    expect(mixedAgents(runs.slice(0, 1))).toBe(false);
  });
});

describe('assignmentRows', () => {
  it('maps names to ids and drops stages with no agent', () => {
    const rows = assignmentRows([{ stage: 'spec', agent: 'alpha', human: true }, { stage: 'design', agent: '', human: false }], [{ id: 'a1', name: 'alpha' }]);
    expect(rows).toEqual([{ stage: 'spec', agentId: 'a1', human: true }]);
  });
});
