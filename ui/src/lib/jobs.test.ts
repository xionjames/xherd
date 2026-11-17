import { describe, it, expect } from 'vitest';
import { initials, jobGroup, track } from './jobs';
import type { Job, FlowInfo } from './types';

const job = (o: Partial<Job>): Job => ({ id: 'j', name: 'j', status: 'running', ...o }) as Job;

describe('initials', () => {
  it('takes the first letters of two words, else the first two characters', () => {
    expect(initials('el-chino')).toBe('EC');
    expect(initials('archie')).toBe('AR');
    expect(initials(undefined)).toBe('–');
  });
});

describe('jobGroup', () => {
  it('sorts by what the user must do', () => {
    expect(jobGroup(job({ status: 'waiting-input' }))).toBe('needs');
    expect(jobGroup(job({ status: 'running' }))).toBe('running');
    expect(jobGroup(job({ status: 'watching' }))).toBe('waiting');
    expect(jobGroup(job({ status: 'queued' }))).toBe('waiting');
    expect(jobGroup(job({ status: 'cancelled' }))).toBe('done');
  });
});

describe('track', () => {
  const flows: FlowInfo[] = [{ pack: 'xherd', flow: 'feature', stages: [
    { id: 'spec', role: 'spec', class: 'x', gate: true },
    { id: 'implement', role: 'implement', class: 'x', gate: true },
  ] }];
  it('has a segment per stage with its agent', () => {
    const j = job({ flow: 'feature', pack: 'xherd', stage: 'implement', visitedStages: ['spec', 'implement'],
      agents: { a1: { id: 'a1', name: 'archie' } as never }, stageAgents: { spec: { agent: 'a1', human: false }, implement: { agent: 'a1', human: false } } });
    const t = track(j, flows);
    expect(t.map((s) => [s.stage, s.state, s.initials])).toEqual([['spec', 'ok', 'AR'], ['implement', 'cur', 'AR']]);
  });
  it('is empty without a workflow', () => expect(track(job({}), flows)).toEqual([]));
});

import { filterJobs, waitingDetail } from './jobs';
describe('counters', () => {
  const l = [job({ name: 'a', status: 'running' }), job({ name: 'b', status: 'queued' }), job({ name: 'c', status: 'watching' }), job({ name: 'd', status: 'done' })];
  it('all hides done; a group shows only itself', () => {
    expect(filterJobs(l, 'all').map((j) => j.name)).toEqual(['a', 'b', 'c']);
    expect(filterJobs(l, 'done').map((j) => j.name)).toEqual(['d']);
    expect(filterJobs(l, 'waiting').map((j) => j.name)).toEqual(['b', 'c']);
  });
  it('says what waits', () => expect(waitingDetail(l)).toBe('1 queued · 1 watching PR'));
});
