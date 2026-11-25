import { beforeEach, describe, expect, it } from 'vitest';
import { cardButtons, cardFromEvent, cardFromWaiting, cardNote, fold, jobView, linkPaths, resolutionText, runningTool, saveJobView, slashMatches } from './chat';
import type { ChatBody, ChatEvent } from './types';

let seq = 0;
const ev = (body: ChatBody): ChatEvent => ({ seq: ++seq, at: seq, ...body }) as ChatEvent;
const tool = (id: string, status: 'running' | 'ok' | 'error', result?: string) =>
  ev({ kind: 'tool', id, name: 'Bash', summary: `Bash: ${id}`, status, result });

describe('fold (R8)', () => {
  it('keeps the latest event per tool id at the first position', () => {
    const { items } = fold([ev({ kind: 'assistant', text: 'a' }), tool('t1', 'running'), ev({ kind: 'assistant', text: 'b' }), tool('t1', 'ok', 'done')]);
    expect(items.map((i) => i.kind)).toEqual(['assistant', 'tool', 'assistant']);
    expect(items[1]).toMatchObject({ status: 'ok', result: 'done' });
  });
  it('keeps the latest plan apart, drops status and unknown lines', () => {
    const { items, plan } = fold([
      ev({ kind: 'plan', items: [{ text: 'a', status: 'pending' }] }),
      ev({ kind: 'status', state: 'working' }),
      ev({ kind: 'unknown', raw: '{}' }),
      ev({ kind: 'plan', items: [{ text: 'a', status: 'completed' }] }),
    ]);
    expect(items).toEqual([]);
    expect(plan).toEqual([{ text: 'a', status: 'completed' }]);
  });
  it('marks messages xherd typed', () => {
    const { items } = fold([ev({ kind: 'user', text: '/xherd:next', source: 'xherd' }), ev({ kind: 'user', text: 'hi', source: 'user' })]);
    expect(items.map((i) => i.xherd)).toEqual([true, undefined]);
  });
  it('replaces a waiting card with its resolved one', () => {
    const q = { kind: 'permission' as const, id: 'p1', tool: 'Bash', summary: 'ls', options: ['allow' as const, 'deny' as const] };
    const { items } = fold([ev({ ...q, state: 'waiting' }), ev({ ...q, state: 'answered', answer: 'allow', from: 'inbox' })]);
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ state: 'answered', from: 'inbox' });
  });
});

describe('runningTool', () => {
  it('is the last running tool, or null', () => {
    const { items } = fold([tool('a', 'running'), tool('b', 'ok'), tool('c', 'running')]);
    expect(runningTool(items)).toBe('Bash: c');
    expect(runningTool(fold([tool('a', 'ok')]).items)).toBeNull();
  });
});

describe('resolutionText (R23)', () => {
  it('says who answered, or why nobody did', () => {
    expect(resolutionText({ state: 'waiting' })).toBeNull();
    expect(resolutionText({ state: 'answered', answer: 'Allow', from: 'inbox' })).toBe('Allow · in the Inbox');
    expect(resolutionText({ state: 'answered', answer: 'Deny', from: 'chat' })).toBe('Deny · in the chat');
    expect(resolutionText({ state: 'in-terminal' })).toMatch(/terminal/);
    expect(resolutionText({ state: 'timed-out' })).toMatch(/No answer/);
    expect(resolutionText({ state: 'lost' })).toMatch(/restarted/);
  });
});

describe('jobView (R7)', () => {
  beforeEach(() => {
    const m = new Map<string, string>();
    Object.assign(globalThis, {
      localStorage: { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) },
    });
  });
  it('opens on the config view without a choice', () => {
    expect(jobView('a', true, 'chat')).toBe('chat');
    expect(jobView('a', true, 'terminal')).toBe('terminal');
    expect(jobView('a', true, undefined)).toBe('terminal');
  });
  it('remembers the choice per job', () => {
    saveJobView('a', 'terminal');
    expect(jobView('a', true, 'chat')).toBe('terminal');
    expect(jobView('b', true, 'chat')).toBe('chat');
  });
  it('is the terminal for a runtime without chat, whatever was chosen', () => {
    saveJobView('a', 'chat');
    expect(jobView('a', false, 'chat')).toBe('terminal');
  });
  it('survives blocked storage', () => {
    Object.assign(globalThis, { localStorage: { getItem: () => { throw new Error('no'); }, setItem: () => { throw new Error('no'); } } });
    expect(jobView('a', true, 'chat')).toBe('chat');
    expect(() => saveJobView('a', 'chat')).not.toThrow();
  });
});

describe('slashMatches (R12)', () => {
  const cmds = ['/xherd:next', '/xherd:review', '/compact', '/clear'];
  it('filters by prefix while the first word is typed', () => {
    expect(slashMatches('/', cmds)).toEqual(cmds);
    expect(slashMatches('/c', cmds)).toEqual(['/compact', '/clear']);
    expect(slashMatches('/XHERD:r', cmds)).toEqual(['/xherd:review']);
  });
  it('is empty for plain text or once arguments start', () => {
    expect(slashMatches('hello', cmds)).toEqual([]);
    expect(slashMatches('/compact now', cmds)).toEqual([]);
    expect(slashMatches('', cmds)).toEqual([]);
  });
});

describe('linkPaths (R9)', () => {
  const href = (p: string) => `/f?path=${p}`;
  const wd = '/w/job';
  it('links a path under the worktree', () => {
    expect(linkPaths('<p>see /w/job/src/a.rs now</p>', wd, href)).toBe(
      '<p>see <a href="/f?path=/w/job/src/a.rs" download>/w/job/src/a.rs</a> now</p>',
    );
  });
  it('leaves other paths, the worktree itself and escapes out of it', () => {
    for (const t of ['/etc/passwd', '/w/job', '/w/job/../x/a', '/w/jobs/a']) {
      expect(linkPaths(`<p>${t}</p>`, wd, href)).toBe(`<p>${t}</p>`);
    }
  });
  it('does not touch tags, existing links or the path of a URL', () => {
    expect(linkPaths('<a href="/w/job/a.rs">/w/job/a.rs</a>', wd, href)).toBe('<a href="/w/job/a.rs">/w/job/a.rs</a>');
    expect(linkPaths('<p>https://x.io/w/job/a.rs</p>', wd, href)).toBe('<p>https://x.io/w/job/a.rs</p>');
  });
  it('does not take the closing parenthesis or a final dot', () => {
    expect(linkPaths('<p>(/w/job/a.rs).</p>', wd, href)).toContain('>/w/job/a.rs</a>).');
  });
});

describe('cards (R23)', () => {
  const perm = (extra: object = {}) =>
    ev({ kind: 'permission', id: 'p1', tool: 'Bash', summary: 'ls', options: ['allow', 'allow-always', 'deny'], state: 'waiting', ...extra } as ChatBody);
  const question = (answerable: boolean, extra: object = {}) =>
    ev({
      kind: 'question', id: 'q1', answerable, state: 'waiting',
      questions: [{ question: 'Which?', header: 'Pick', multiSelect: false, options: [{ label: 'A', description: '' }, { label: 'B', description: '' }] }],
      ...extra,
    } as ChatBody);

  it('offers one button per option while a permission waits', () => {
    const c = cardFromEvent(perm())!;
    expect(cardButtons(c)).toEqual([
      { label: 'Allow', value: 'allow' }, { label: 'Always allow', value: 'allow-always' }, { label: 'Deny', value: 'deny' },
    ]);
    expect(cardNote(c)).toBeNull();
  });
  it('offers the question options by index only when answerable', () => {
    expect(cardButtons(cardFromEvent(question(true))!)).toEqual([{ label: 'A', value: 0 }, { label: 'B', value: 1 }]);
    const c = cardFromEvent(question(false))!;
    expect(cardButtons(c)).toEqual([]);
    expect(cardNote(c)).toBe('Answer in the terminal.');
  });
  it('has no buttons once resolved and says how it ended', () => {
    const a = cardFromEvent(perm({ state: 'answered', answer: 'allow', from: 'inbox' }))!;
    expect(cardButtons(a)).toEqual([]);
    expect(cardNote(a)).toBe('allow · in the Inbox');
    expect(cardNote(cardFromEvent(perm({ state: 'timed-out' }))!)).toContain('No answer in time');
    expect(cardNote(cardFromEvent(question(true, { state: 'in-terminal' }))!)).toBe('Answered in the terminal');
  });
  it('builds a waiting card from Job.waiting', () => {
    const c = cardFromWaiting({ id: 'p2', kind: 'permission', since: 1, tool: 'Edit', summary: 'a.rs', options: ['allow', 'deny'], answerable: true });
    expect(c).toMatchObject({ title: 'Edit', text: 'a.rs' });
    expect(cardButtons(c).map((b) => b.value)).toEqual(['allow', 'deny']);
    const q = cardFromWaiting({ id: 'q2', kind: 'question', since: 1, summary: 'Which?', answerable: false, questions: [] });
    expect(cardNote(q)).toBe('Answer in the terminal.');
  });
  it('is not a card for other events', () => {
    expect(cardFromEvent(ev({ kind: 'assistant', text: 'x' }))).toBeNull();
  });
});
