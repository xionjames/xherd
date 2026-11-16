import { describe, expect, it } from 'vitest';
import { boardLane, laneOrder, todoLane } from './board';
import type { Job } from './types';

const a = (over: Partial<Job>): Job =>
  ({ id: 'x', name: 'fix-login', branch: 'b', status: 'running', createdAt: 0, statusSince: 0, lastActivity: 0, repo: '/w/app', ...over });

describe('board lanes', () => {
  it('?lanes=flow groups cards by the agent workflow', () => {
    expect(boardLane(a({ flow: 'bugfix' }), 'flow')).toBe('Bugfix');
    expect(boardLane(a({ flow: 'feature' }), 'flow')).toBe('Feature');
    expect(boardLane(a({}), 'flow')).toBe('No workflow');
  });

  it('the other switches keep their lanes', () => {
    expect(boardLane(a({ flow: 'bugfix' }), 'none')).toBe('');
    expect(boardLane(a({ flow: 'bugfix' }), 'agent')).toBe('fix-login');
    expect(boardLane(a({ flow: 'bugfix' }), 'repo')).toBe('app');
  });

  it('TODOs keep their own lane, last', () => {
    expect(todoLane('flow')).toBe('TODOs');
    expect(todoLane('none')).toBe('');
    expect(laneOrder(['TODOs', 'No workflow', 'Feature', 'Bugfix', 'Feature'])).toEqual(['Bugfix', 'Feature', 'No workflow', 'TODOs']);
  });
});

describe('board lanes by saved agent', () => {
  it('?lanes=agent-def groups by the job\'s agent snapshot, No agent last', () => {
    const withAgent = a({ agent: { id: 'g1', name: 'reviewer', version: 2 } });
    expect(boardLane(withAgent, 'agent-def')).toBe('reviewer');
    expect(boardLane(a({}), 'agent-def')).toBe('No agent');
    expect(laneOrder(['TODOs', 'No agent', 'zed', 'reviewer'])).toEqual(['reviewer', 'zed', 'No agent', 'TODOs']);
  });
});
