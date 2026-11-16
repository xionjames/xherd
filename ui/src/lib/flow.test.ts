import { describe, expect, it } from 'vitest';
import { conflictsAction, prGate, stageLabel, stageNow, steps } from './flow';
import type { Job, FlowInfo } from './types';

describe('steps', () => {
  const f: FlowInfo = {
    pack: 'xherd', flow: 'feature',
    stages: ['spec', 'design', 'tasks', 'implement'].map((id) => ({ id, role: id as never, class: 'routine', gate: true })),
  };
  const a = (over: Partial<Job>): Job => ({ id: 'x', name: 'x', branch: 'x', status: 'waiting-input', createdAt: 0, statusSince: 0, lastActivity: 0, ...over });
  it('marks stages never entered as skipped', () => {
    const s = steps(a({ stage: 'tasks', visitedStages: ['spec', 'tasks'], gate: { kind: 'question', text: '?' } }), f);
    expect(s.map((x) => x.state)).toEqual(['ok', 'skip', 'now', 'todo']);
  });
  it('without a record, everything before is done', () => {
    expect(steps(a({ stage: 'tasks', status: 'running' }), f).map((x) => x.state)).toEqual(['ok', 'ok', 'cur', 'todo']);
  });

  const pr: FlowInfo = {
    pack: 'xherd', flow: 'feature',
    stages: [['review', 'review'], ['ship', 'ship'], ['pr-review', 'pr'], ['pr', 'pr']].map(([id, role]) => ({ id, role: role as never, class: 'routine', gate: true })),
  };
  it('a watching agent is at the last PR stage; a PR review not held yet stays to do', () => {
    const s = steps(a({ stage: 'ship', status: 'watching', visitedStages: ['review', 'ship'] }), pr);
    expect(s.map((x) => x.state)).toEqual(['ok', 'ok', 'todo', 'cur']);
  });
  it('a watching agent back from an earlier stage keeps the later stages it went through', () => {
    const s = steps(a({ stage: 'review', status: 'watching', visitedStages: ['review', 'ship', 'pr-review'] }), pr);
    expect(s.map((x) => x.state)).toEqual(['ok', 'ok', 'ok', 'cur']);
  });
  it('a done agent has every stage behind it', () => {
    const s = steps(a({ stage: 'ship', status: 'done', visitedStages: ['review', 'ship'] }), pr);
    expect(s.map((x) => x.state)).toEqual(['ok', 'ok', 'skip', 'skip']);
  });
});

describe('stageLabel', () => {
  const st = (id: string, role: string) => ({ id, role: role as never, class: 'routine', gate: true });
  it('names a stage by its role when the id is the role', () => {
    expect(stageLabel(st('implement', 'implement'))).toBe('Build');
    expect(stageLabel(st('pr', 'pr'))).toBe('PR');
  });
  it('names a stage by its id otherwise, so two stages of one role differ', () => {
    expect(stageLabel(st('pr-review', 'pr'))).toBe('PR Review');
    expect(stageLabel(st('security_scan', 'review'))).toBe('Security Scan');
  });
  it('prefers the label the pack sets', () => {
    expect(stageLabel({ ...st('implement', 'implement'), label: 'Fix' })).toBe('Fix');
    expect(stageLabel(st('reproduce', 'implement'))).toBe('Reproduce');
  });
});

describe('stageNow', () => {
  const f: FlowInfo = {
    pack: 'xherd', flow: 'feature',
    stages: [
      { id: 'implement', role: 'implement', class: 'code', gate: true },
      { id: 'ship', role: 'ship', class: 'critical', gate: true },
      { id: 'pr-review', role: 'pr', class: 'human-only', gate: true },
    ],
  };
  const a = (over: Partial<Job>): Job =>
    ({ id: 'x', name: 'x', branch: 'x', status: 'idle', createdAt: 0, statusSince: 0, lastActivity: 0, pack: 'xherd', flow: 'feature', ...over });
  it('names the stage and what waits at its gate', () => {
    expect(stageNow(a({ stage: 'ship', gate: { kind: 'docs', stage: 'ship', role: 'ship', files: [{ path: '.xherd/spec/pr.md', sha256: 's' }] } }), [f]))
      .toEqual({ name: 'Ship', waits: 'pr.md' });
    expect(stageNow(a({ stage: 'implement', gate: { kind: 'commit', id: 'C05' } as never }), [f])).toEqual({ name: 'Build', waits: 'C05' });
    expect(stageNow(a({ stage: 'pr-review', status: 'running' }), [f])).toEqual({ name: 'PR Review', waits: undefined });
  });
  it('has nothing to say without a stage, and falls back to the id', () => {
    expect(stageNow(a({}), [f])).toBeUndefined();
    expect(stageNow(a({ stage: 'odd' }), [f])).toEqual({ name: 'odd', waits: undefined });
  });
});

describe('prGate', () => {
  it('is the docs gate of a PR stage only', () => {
    const docs = (role: string) => ({ kind: 'docs' as const, stage: 's', role: role as never, files: [] });
    expect(prGate(docs('pr'))).toBe(true);
    expect(prGate(docs('ship'))).toBe(false);
    expect(prGate({ kind: 'question', text: '?' })).toBe(false);
    expect(prGate(undefined)).toBe(false);
  });
});

describe('conflictsAction', () => {
  const a = (over: Partial<Job>): Job => ({
    id: 'x', name: 'x', branch: 'x', status: 'watching', createdAt: 0, statusSince: 0, lastActivity: 0,
    flow: 'feature', prUrl: 'https://github.com/o/r/pull/1', prState: 'open', prMergeable: 'no', ...over,
  });
  it('shows for a conflicting PR of a flow agent', () => {
    expect(conflictsAction(a({}))).toEqual({ show: true, enabled: true });
    expect(conflictsAction(a({ prMergeable: undefined, prConflict: true })).show).toBe(true);
  });
  it('hides without a PR, without a conflict, for flow none or a finished PR', () => {
    expect(conflictsAction(a({ prUrl: undefined })).show).toBe(false);
    expect(conflictsAction(a({ prMergeable: 'yes' })).show).toBe(false);
    expect(conflictsAction(a({ prMergeable: 'unknown' })).show).toBe(false);
    expect(conflictsAction(a({ flow: undefined })).show).toBe(false);
    expect(conflictsAction(a({ prState: 'merged' })).show).toBe(false);
    expect(conflictsAction(a({ worktreeRemovedAt: 1 })).show).toBe(false);
  });
  it('is enabled for stopped agents with no gate only', () => {
    for (const status of ['watching', 'paused', 'error', 'idle'] as const) {
      expect(conflictsAction(a({ status })).enabled).toBe(true);
    }
    const running = conflictsAction(a({ status: 'running' }));
    expect(running).toEqual({ show: true, enabled: false, why: 'Wait until the agent stops.' });
    const gated = conflictsAction(a({ status: 'idle', gate: { kind: 'question', text: '?' } }));
    expect(gated).toEqual({ show: true, enabled: false, why: 'Decide the open gate first.' });
  });
});
