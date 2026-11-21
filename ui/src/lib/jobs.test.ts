import { describe, it, expect } from 'vitest';
import { initials, jobGroup, track, segTitle } from './jobs';
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
  it('marks the stages with a fresh-context check in the segment and its title', () => {
    const j = job({ flow: 'feature', pack: 'xherd', stage: 'spec', freshChecks: ['spec'] });
    const t = track(j, flows);
    expect(t.map((s) => !!s.fresh)).toEqual([true, false]);
    expect(segTitle(t[0])).toContain(' · fresh-context check');
    expect(segTitle(t[1])).not.toContain('fresh-context');
  });
});

import { sortJobs } from './jobs';
describe('sortJobs', () => {
  const names = (l: Job[]) => l.map((j) => j.name);
  it('puts the newest first outside Needs you', () => {
    const l = [job({ name: 'old', createdAt: 1 }), job({ name: 'new', createdAt: 3 }), job({ name: 'mid', status: 'queued', createdAt: 2 })];
    expect(names(sortJobs(l))).toEqual(['new', 'mid', 'old']);
  });
  it('puts Needs you first, longest waiting first', () => {
    const l = [
      job({ name: 'run', createdAt: 9 }),
      job({ name: 'late', status: 'waiting-input', statusSince: 20, createdAt: 1 }),
      job({ name: 'early', status: 'waiting-input', statusSince: 10, createdAt: 2 }),
    ];
    expect(names(sortJobs(l))).toEqual(['early', 'late', 'run']);
  });
  it('breaks ties by name', () => {
    const l = [job({ name: 'b', createdAt: 5 }), job({ name: 'a', createdAt: 5 })];
    expect(names(sortJobs(l))).toEqual(['a', 'b']);
  });
  it('does not change its input', () => {
    const l = [job({ name: 'x', createdAt: 1 }), job({ name: 'y', createdAt: 2 })];
    sortJobs(l);
    expect(names(l)).toEqual(['x', 'y']);
  });
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
