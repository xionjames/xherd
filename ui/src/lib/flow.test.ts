import { describe, expect, it } from 'vitest';
import { stageLabel, steps } from './flow';
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
