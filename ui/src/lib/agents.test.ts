import { describe, expect, it } from 'vitest';
import {
  agentFor,
  FORM_DEFAULTS, agentLane, agentLaneOrder, canMarkDone, changedSince, fillFromAgent, fillFromJob, fromForm, pickable,
  profileMarks, slugify, suggestionOpen, toForm, unmark, versionDiff, waitsForAgent, NO_AGENT_LANE,
} from './agents';
import type { AgentDef, AgentView, Job } from './types';

const def = (over: Partial<AgentDef> = {}): AgentDef => ({
  id: 'ag1', name: 'reviewer', description: '', version: 1, runtime: 'claude-code',
  maxLiveJobs: 1, createdAt: 0, updatedAt: 0, ...over,
});
const view = (over: Partial<AgentView> = {}): AgentView => ({ ...def(), status: 'idle', liveJobs: 0, lastJob: null, tokensToday: 0, ...over });

describe('fillFromAgent', () => {
  it('fills the fields the agent sets and marks them', () => {
    const a = def({ runtime: 'opencode', models: { opencode: 'big', 'claude-code': 'opus' }, autonomy: 'important', permissionMode: 'auto' });
    const { form, marks } = fillFromAgent(a, FORM_DEFAULTS);
    expect(form).toEqual({ runtime: 'opencode', model: 'big', autonomy: 'important', mode: 'auto' });
    expect(marks.sort()).toEqual(['autonomy', 'mode', 'model', 'runtime']);
  });

  it('leaves what the agent does not set, and marks only what it sets', () => {
    const { form, marks } = fillFromAgent(def(), { ...FORM_DEFAULTS, model: 'mine' });
    expect(form.model).toBe('mine');
    expect(form.autonomy).toBe('ask');
    expect(marks.sort()).toEqual(['runtime']);
  });

  it('a different pick resets what the previous agent filled, and keeps the user\'s own', () => {
    const first = fillFromAgent(def({ runtime: 'codex', autonomy: 'important' }), FORM_DEFAULTS);
    const typed = { ...first.form, mode: 'ask' as const };
    const second = fillFromAgent(def({ name: 'b' }), typed, first.marks);
    expect(second.form).toMatchObject({ runtime: 'claude-code', autonomy: 'ask', mode: 'ask' });
  });

  it('"No agent" gives the defaults back', () => {
    const first = fillFromAgent(def({ autonomy: 'important' }), FORM_DEFAULTS);
    const none = fillFromAgent(undefined, first.form, first.marks);
    expect(none.form).toEqual(FORM_DEFAULTS);
    expect(none.marks).toEqual([]);
  });

  it('editing a field removes its mark', () => {
    expect(unmark(['model', 'mode'], 'model')).toEqual(['mode']);
  });
});

describe('pickable', () => {
  it('hides archived and broken agents', () => {
    expect(pickable(view())).toBe(true);
    expect(pickable(view({ status: 'archived' }))).toBe(false);
    expect(pickable(view({ broken: 'the runtime is switched off' }))).toBe(false);
  });
});

describe('profileMarks', () => {
  const p = { name: 'api', autonomy: 'ask', permissionMode: 'edits', sensitivePaths: ['a', 'b'], maxDiffLines: 500, jobTokenBudget: 1000 };

  it('autonomy and mode are "at most", lowered when the agent is looser', () => {
    const m = profileMarks({ autonomy: 'important', permissionMode: 'all' }, p);
    expect(m.autonomy).toMatchObject({ kind: 'caps', lowered: true, text: 'at most ask, so important becomes ask' });
    expect(m.permissionMode).toMatchObject({ kind: 'caps', lowered: true });
    const ok = profileMarks({ autonomy: 'ask', permissionMode: 'edits' }, p);
    expect(ok.autonomy?.lowered).toBe(false);
    expect(ok.permissionMode?.lowered).toBe(false);
  });

  it('an agent with no value is capped without being lowered', () => {
    expect(profileMarks({}, p).autonomy).toMatchObject({ kind: 'caps', lowered: false, text: 'at most ask' });
  });

  it('paths are added to, limits take the smaller value', () => {
    const m = profileMarks({ maxDiffLines: 900, jobTokenBudget: 50 }, p);
    expect(m.sensitivePaths).toMatchObject({ kind: 'adds', text: "added to the profile's 2 paths" });
    expect(m.maxDiffLines).toMatchObject({ kind: 'tightens', lowered: true });
    expect(m.jobTokenBudget).toMatchObject({ kind: 'tightens', lowered: false });
  });

  it('a profile that sets nothing, or no profile, marks nothing', () => {
    expect(profileMarks({ autonomy: 'ask' }, { name: 'x' })).toEqual({});
    expect(profileMarks({ autonomy: 'ask' }, null)).toEqual({});
  });
});

describe('versionDiff', () => {
  it('lists the fields that changed, with empty text for unset ones', () => {
    const a = def({ autonomy: 'ask', instructions: 'old' });
    const b = def({ version: 2, updatedAt: 9, autonomy: 'important', models: { 'claude-code': 'opus' } });
    expect(versionDiff(a, b)).toEqual([
      { field: 'models', label: 'Models', from: '', to: 'claude-code: opus' },
      { field: 'autonomy', label: 'Autonomy', from: 'ask', to: 'important' },
      { field: 'instructions', label: 'Instructions', from: 'old', to: '' },
    ]);
  });

  it('is empty for equal settings; the version and dates are not settings', () => {
    expect(versionDiff(def(), def({ version: 5, updatedAt: 99, archivedAt: 3 }))).toEqual([]);
  });

  it('shows the fallback runtime', () => {
    const d = versionDiff(def(), def({ fallbackRuntime: 'opencode' }));
    expect(d).toEqual([{ field: 'fallbackRuntime', label: 'Fallback runtime', from: '', to: 'opencode' }]);
  });
});

describe('agentLane', () => {
  it('is the snapshot name, else No agent, which sorts last', () => {
    expect(agentLane({ agents: { x: { id: 'x', name: 'reviewer', version: 1 } }, currentAgent: 'x' })).toBe('reviewer');
    expect(agentLane({})).toBe(NO_AGENT_LANE);
    expect(agentLaneOrder([NO_AGENT_LANE, 'zed', 'alpha', 'zed'])).toEqual(['alpha', 'zed', NO_AGENT_LANE]);
  });
});

describe('waitsForAgent', () => {
  const j = (status: string, id = 'ag1') => ({ status: status as never, agents: { [id]: { id, name: 'a', version: 1 } }, currentAgent: id });
  it('a queued job whose agent is at its limit waits for it', () => {
    expect(waitsForAgent(j('queued'), [view({ liveJobs: 1, maxLiveJobs: 1 })])).toBe(true);
    expect(waitsForAgent(j('queued'), [view({ liveJobs: 1, maxLiveJobs: 2 })])).toBe(false);
  });
  it('not for other statuses, jobs without an agent, or an agent that is gone', () => {
    expect(waitsForAgent(j('running'), [view({ liveJobs: 1 })])).toBe(false);
    expect(waitsForAgent({ status: 'queued' }, [view({ liveJobs: 1 })])).toBe(false);
    expect(waitsForAgent(j('queued', 'gone'), [view({ liveJobs: 1 })])).toBe(false);
  });
});

describe('changedSince', () => {
  it('is true only when the agent is newer than the job\'s snapshot', () => {
    const job = { agents: { ag1: { id: 'ag1', name: 'a', version: 2 } }, currentAgent: 'ag1' };
    expect(changedSince(job, { version: 3 })).toBe(true);
    expect(changedSince(job, { version: 2 })).toBe(false);
    expect(changedSince({}, { version: 9 })).toBe(false);
    expect(changedSince(job, undefined)).toBe(false);
  });
});

describe('the edit form', () => {
  it('round-trips an agent, keeping other runtimes\' models and the skills', () => {
    const a = def({ models: { 'claude-code': 'opus', opencode: 'big' }, skills: ['lint'], autonomy: 'ask', sensitivePaths: ['a', 'b'], maxDiffLines: 300, fallbackRuntime: 'opencode' });
    const body = fromForm(toForm(a), a);
    expect(body).toMatchObject({ name: 'reviewer', runtime: 'claude-code', models: { 'claude-code': 'opus', opencode: 'big' }, skills: ['lint'], autonomy: 'ask', sensitivePaths: ['a', 'b'], maxDiffLines: 300, fallbackRuntime: 'opencode', maxLiveJobs: 1 });
  });

  it('an emptied field is cleared, a cleared model leaves the others', () => {
    const a = def({ models: { 'claude-code': 'opus', opencode: 'big' }, autonomy: 'important', instructions: 'x', fallbackRuntime: 'opencode' });
    const f = { ...toForm(a), autonomy: '', instructions: ' ', fallbackRuntime: '', model: '' };
    const body = fromForm(f, a);
    expect(body.autonomy).toBeUndefined();
    expect(body.instructions).toBeUndefined();
    expect(body.fallbackRuntime).toBeUndefined();
    expect(body.models).toEqual({ opencode: 'big' });
  });

  it('numbers become numbers and a bad live-jobs count falls back to 1', () => {
    const f = { ...toForm(), jobTokenBudget: '5000', maxDiffLines: '200', maxLiveJobs: 'x' };
    expect(fromForm(f)).toMatchObject({ jobTokenBudget: 5000, maxDiffLines: 200, maxLiveJobs: 1 });
  });
});

describe('suggestions (R30…R32)', () => {
  const job = (over: Partial<Job> = {}): Job => ({ id: 'j1', name: 'login', status: 'done', createdAt: 0, statusSince: 0, lastActivity: 0, branch: 'login', ...over } as Job);

  it('slugify makes a valid name', () => {
    expect(slugify('My Shop_API!')).toBe('my-shop-api');
    expect(slugify('a'.repeat(60))).toHaveLength(41);
    expect(slugify('--x--')).toBe('x');
  });

  it('is open only for a job with no agent, with a suggestion that was not dismissed', () => {
    const s = { reason: 'done', at: 1, dismissed: false };
    expect(suggestionOpen(job({ suggestion: s }))).toBe(true);
    expect(suggestionOpen(job({ suggestion: { ...s, dismissed: true } }))).toBe(false);
    expect(suggestionOpen(job())).toBe(false);
    expect(suggestionOpen(job({ suggestion: s, agents: { a: { id: 'a', name: 'a', version: 1 } } }))).toBe(false);
  });

  it('Mark done is for a job with no workflow between turns', () => {
    expect(canMarkDone({ flow: undefined, status: 'idle' })).toBe(true);
    expect(canMarkDone({ flow: 'none', status: 'paused' })).toBe(true);
    expect(canMarkDone({ flow: 'feature', status: 'idle' })).toBe(false);
    expect(canMarkDone({ flow: undefined, status: 'running' })).toBe(false);
    expect(canMarkDone({ flow: undefined, status: 'error' })).toBe(false);
  });

  it('fills the form from the job, then from its draft', () => {
    const j = job({ repo: '/ws/repos/Shop API', flow: 'feature', pack: 'xherd', runtime: 'opencode', model: 'big', autonomy: 'important', permissionMode: 'auto' });
    expect(fillFromJob(j)).toMatchObject({ name: 'shop-api-feature', description: 'feature work on Shop API', runtime: 'opencode', model: 'big', autonomy: 'important', permissionMode: 'auto', instructions: '' });
    const d = fillFromJob({ ...j, agentDraft: { name: 'shop', description: 'Shop work', instructions: 'Run the linter.', at: 1 } });
    expect(d).toMatchObject({ name: 'shop', description: 'Shop work', instructions: 'Run the linter.' });
    expect(fillFromJob({ ...j, agentDraft: { name: 'Bad Name', description: '', instructions: '', at: 1 } }).name).toBe('shop-api-feature');
  });
});

describe('agentFor', () => {
  const job = { agents: { a1: { name: 'peter' }, a2: { name: 'ana' } }, stageAgents: { spec: { agent: 'a1', human: false }, review: { agent: 'a2', human: false } }, currentAgent: 'a1' } as unknown as Job;
  it('names the agent of the stage', () => {
    expect(agentFor(job, 'review')).toBe('ana');
    expect(agentFor(job, 'spec')).toBe('peter');
  });
  it('has none for a stage without one', () => expect(agentFor(job, 'ship')).toBeUndefined());
  it('uses the current agent for a job with no workflow', () => expect(agentFor(job)).toBe('peter'));
});
