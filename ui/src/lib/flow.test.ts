import { describe, expect, it } from 'vitest';
import { stageLabel, stageNow, steps } from './flow';
import type { Agent, FlowInfo } from './types';

describe('steps', () => {
  const f: FlowInfo = {
    pack: 'xherd', flow: 'feature',
    stages: ['spec', 'design', 'tasks', 'implement'].map((id) => ({ id, role: id as never, class: 'routine', gate: true })),
  };
  const a = (over: Partial<Agent>): Agent => ({ id: 'x', name: 'x', ticketKey: 'X', status: 'waiting-input', createdAt: 0, statusSince: 0, lastActivity: 0, ...over });
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
  const a = (over: Partial<Agent>): Agent =>
    ({ id: 'x', name: 'x', ticketKey: 'X', status: 'idle', createdAt: 0, statusSince: 0, lastActivity: 0, pack: 'xherd', flow: 'feature', ...over });
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
