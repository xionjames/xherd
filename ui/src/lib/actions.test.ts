import { describe, it, expect } from 'vitest';
import {
  actionOfTab, defaultOpen, groups, headerMarks, headerText, lastResult, needsApproval, saveCard, tabId, visible,
} from './actions';
import type { JobAction } from './types';

const act = (id: string, o: Partial<JobAction> & { kind?: 'service' | 'task'; label?: string } = {}): JobAction => ({
  def: { id, label: o.label ?? id, description: '', run: 'x', cwd: '', kind: o.kind ?? 'task', icon: '' },
  source: o.source ?? 'job',
  approved: 'approved' in o ? o.approved : 'h',
  last: o.last,
  succeeded: o.succeeded ?? false,
});
const run = (state: 'running' | 'exited' | 'stopped' | 'lost', exit?: number) => ({ startedAt: 1, state, exit });

describe('groups', () => {
  it('splits services and tasks, running first, then by label', () => {
    const g = groups([
      act('b', { kind: 'service', label: 'B' }),
      act('a', { kind: 'service', label: 'A' }),
      act('c', { kind: 'service', label: 'C', last: run('running') }),
      act('t', { label: 'Test' }),
    ]);
    expect(g.service.map(a => a.def.id)).toEqual(['c', 'a', 'b']);
    expect(g.task.map(a => a.def.id)).toEqual(['t']);
    expect(groups(undefined)).toEqual({ service: [], task: [] });
  });
});

describe('defaultOpen and visible', () => {
  it('opens up to 4 entries and folds beyond 8', () => {
    expect(defaultOpen(4)).toBe(true);
    expect(defaultOpen(5)).toBe(false);
    const list = Array.from({ length: 10 }, (_, i) => act(`a${i}`));
    expect(visible(list, false)).toEqual({ shown: list.slice(0, 8), hidden: 2 });
    expect(visible(list, true)).toEqual({ shown: list, hidden: 0 });
    expect(visible(list.slice(0, 8), false).hidden).toBe(0);
  });
});

describe('header', () => {
  it('marks running services and actions awaiting approval', () => {
    const l = [act('a', { kind: 'service', last: run('running') }), act('b', { kind: 'service', approved: undefined })];
    expect(headerMarks(l)).toEqual({ running: 1, approval: 1 });
    expect(headerText('service', l)).toBe('Services · 2 · 1 running');
    expect(headerText('task', [act('t'), act('u')])).toBe('Actions · 2');
  });
});

describe('lastResult', () => {
  it('says it with a mark and a word', () => {
    expect(lastResult(act('a', { approved: undefined })).text).toBe('Needs approval');
    expect(lastResult(act('a')).text).toBe('Not run');
    expect(lastResult(act('a', { last: run('running') })).tone).toBe('run');
    expect(lastResult(act('a', { last: run('exited', 0) }))).toEqual({ icon: '✓', text: 'Exited 0', tone: 'ok' });
    expect(lastResult(act('a', { last: run('exited', 2) }))).toEqual({ icon: '✗', text: 'Exited 2', tone: 'fail' });
    expect(lastResult(act('a', { last: run('stopped') })).text).toBe('Stopped');
    expect(lastResult(act('a', { last: run('lost') })).tone).toBe('fail');
  });
  it('needsApproval follows the approved hash', () => {
    expect(needsApproval(act('a', { approved: undefined }))).toBe(true);
    expect(needsApproval(act('a'))).toBe(false);
  });
});

describe('tab ids and the save card', () => {
  it('maps act-<id> both ways', () => {
    expect(tabId('dev')).toBe('act-dev');
    expect(actionOfTab('act-dev')).toBe('dev');
    expect(actionOfTab('act-')).toBeNull();
    expect(actionOfTab('diff')).toBeNull();
    expect(actionOfTab(null)).toBeNull();
  });
  it('lists the offered actions unless dismissed', () => {
    const l = [act('a'), act('b'), act('c')];
    expect(saveCard(l, ['c', 'a'], false).map(a => a.def.id)).toEqual(['a', 'c']);
    expect(saveCard(l, ['a'], true)).toEqual([]);
    expect(saveCard(undefined, ['a'], false)).toEqual([]);
  });
});
