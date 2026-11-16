import { describe, expect, it } from 'vitest';
import {
  FORM_DEFAULTS, agentLane, agentLaneOrder, changedSince, fillFromAgent, fromForm, pickable,
  profileMarks, toForm, unmark, versionDiff, waitsForAgent, NO_AGENT_LANE,
} from './agents';
import type { AgentDef, AgentView } from './types';

const def = (over: Partial<AgentDef> = {}): AgentDef => ({
  id: 'ag1', name: 'reviewer', description: '', version: 1, runtime: 'claude-code',
  workflow: { pack: 'xherd', flow: 'feature' }, maxLiveJobs: 1, createdAt: 0, updatedAt: 0, ...over,
});
const view = (over: Partial<AgentView> = {}): AgentView => ({ ...def(), status: 'idle', liveJobs: 0, lastJob: null, tokensToday: 0, ...over });

describe('fillFromAgent', () => {
  it('fills the fields the agent sets and marks them', () => {
    const a = def({ runtime: 'opencode', models: { opencode: 'big', 'claude-code': 'opus' }, autonomy: 'important', permissionMode: 'auto', repo: '/r', baseBranch: 'dev', workflow: { pack: 'xherd', flow: 'bugfix' } });
    const { form, marks } = fillFromAgent(a, FORM_DEFAULTS);
    expect(form).toEqual({ flow: 'bugfix', repo: '/r', base: 'dev', runtime: 'opencode', model: 'big', autonomy: 'important', mode: 'auto' });
    expect(marks.sort()).toEqual(['autonomy', 'base', 'flow', 'mode', 'model', 'repo', 'runtime']);
  });

  it('leaves what the agent does not set, and marks only what it sets', () => {
    const { form, marks } = fillFromAgent(def(), { ...FORM_DEFAULTS, repo: '/mine' });
    expect(form.repo).toBe('/mine');
    expect(form.autonomy).toBe('ask');
    expect(marks.sort()).toEqual(['flow', 'runtime']);
  });

  it('a different pick resets what the previous agent filled, and keeps the user\'s own', () => {
    const first = fillFromAgent(def({ repo: '/a', autonomy: 'important' }), FORM_DEFAULTS);
    const typed = { ...first.form, base: 'mine' };
    const second = fillFromAgent(def({ name: 'b', repo: '/b' }), typed, first.marks);
    expect(second.form).toMatchObject({ repo: '/b', autonomy: 'ask', base: 'mine' });
  });

  it('"No agent" gives the defaults back', () => {
    const first = fillFromAgent(def({ repo: '/a' }), FORM_DEFAULTS);
    const none = fillFromAgent(undefined, first.form, first.marks);
    expect(none.form).toEqual(FORM_DEFAULTS);
    expect(none.marks).toEqual([]);
  });

  it('editing a field removes its mark', () => {
    expect(unmark(['repo', 'flow'], 'repo')).toEqual(['flow']);
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
    const b = def({ version: 2, updatedAt: 9, autonomy: 'important', models: { 'claude-code': 'opus' }, stageOverrides: { review: { human: true } } });
    expect(versionDiff(a, b)).toEqual([
      { field: 'models', label: 'Models', from: '', to: 'claude-code: opus' },
      { field: 'autonomy', label: 'Autonomy', from: 'ask', to: 'important' },
      { field: 'instructions', label: 'Instructions', from: 'old', to: '' },
      { field: 'stageOverrides', label: 'Stages that wait for a person', from: '', to: 'review' },
    ]);
  });

  it('is empty for equal settings; the version and dates are not settings', () => {
    expect(versionDiff(def(), def({ version: 5, updatedAt: 99, archivedAt: 3 }))).toEqual([]);
  });

  it('shows the workflow and the flow none', () => {
    const d = versionDiff(def(), def({ workflow: { pack: 'xherd', flow: 'none' } }));
    expect(d).toEqual([{ field: 'workflow', label: 'Workflow', from: 'feature (xherd)', to: 'no workflow' }]);
  });
});

describe('agentLane', () => {
  it('is the snapshot name, else No agent, which sorts last', () => {
    expect(agentLane({ agent: { id: 'x', name: 'reviewer', version: 1 } })).toBe('reviewer');
    expect(agentLane({})).toBe(NO_AGENT_LANE);
    expect(agentLaneOrder([NO_AGENT_LANE, 'zed', 'alpha', 'zed'])).toEqual(['alpha', 'zed', NO_AGENT_LANE]);
  });
});

describe('waitsForAgent', () => {
  const j = (status: string, id = 'ag1') => ({ status: status as never, agent: { id, name: 'a', version: 1 } });
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
    const job = { agent: { id: 'ag1', name: 'a', version: 2 } };
    expect(changedSince(job, { version: 3 })).toBe(true);
    expect(changedSince(job, { version: 2 })).toBe(false);
    expect(changedSince({}, { version: 9 })).toBe(false);
    expect(changedSince(job, undefined)).toBe(false);
  });
});

describe('the edit form', () => {
  it('round-trips an agent, keeping other runtimes\' models and the skills', () => {
    const a = def({ models: { 'claude-code': 'opus', opencode: 'big' }, skills: ['lint'], autonomy: 'ask', sensitivePaths: ['a', 'b'], maxDiffLines: 300, stageOverrides: { review: { human: true } } });
    const body = fromForm(toForm(a), a);
    expect(body).toMatchObject({ name: 'reviewer', runtime: 'claude-code', models: { 'claude-code': 'opus', opencode: 'big' }, skills: ['lint'], autonomy: 'ask', sensitivePaths: ['a', 'b'], maxDiffLines: 300, stageOverrides: { review: { human: true } }, maxLiveJobs: 1 });
  });

  it('an emptied field is cleared, a cleared model leaves the others', () => {
    const a = def({ models: { 'claude-code': 'opus', opencode: 'big' }, autonomy: 'important', instructions: 'x', repo: '/r' });
    const f = { ...toForm(a), autonomy: '', instructions: ' ', repo: '', model: '' };
    const body = fromForm(f, a);
    expect(body.autonomy).toBeUndefined();
    expect(body.instructions).toBeUndefined();
    expect(body.repo).toBeUndefined();
    expect(body.models).toEqual({ opencode: 'big' });
  });

  it('numbers become numbers and a bad live-jobs count falls back to 1', () => {
    const f = { ...toForm(), jobTokenBudget: '5000', maxDiffLines: '200', maxLiveJobs: 'x' };
    expect(fromForm(f)).toMatchObject({ jobTokenBudget: 5000, maxDiffLines: 200, maxLiveJobs: 1 });
  });
});
