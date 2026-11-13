import { describe, expect, it, vi } from 'vitest';
vi.mock('./store.svelte', () => ({ agentByName: () => undefined }));
vi.mock('./router.svelte', () => ({ go: () => {} }));
import { addNotice, notices, settle } from './notices.svelte';
import type { Agent } from './types';

const agent = (status: Agent['status'], gate?: Agent['gate']) =>
  ({ id: 'A1', name: 'a', status, gate }) as Agent;
const read = (kind: string) => notices.list.find((n) => n.agentId === 'A1' && n.kind === kind)!.read;

describe('settle', () => {
  it('marks read the notices whose wait is over, and only those', () => {
    notices.list = [];
    for (const k of ['gate', 'blocked', 'idle', 'pr']) addNotice('A1', k, k);
    addNotice('B2', 'gate', 'other agent');
    const gate = { kind: 'question', text: 'q' } as const;

    settle(agent('blocked', gate)); // still blocked at its gate
    expect(['gate', 'blocked', 'idle', 'pr'].map(read)).toEqual([false, false, true, false]);

    settle(agent('running', gate)); // changes requested: running again
    expect(['gate', 'blocked', 'pr'].map(read)).toEqual([true, true, false]);
    expect(notices.list.find((n) => n.agentId === 'B2')!.read).toBe(false);
  });

  it('marks a gate notice read once the gate is approved', () => {
    notices.list = [];
    addNotice('A1', 'gate', 'held');
    settle(agent('waiting-input', { kind: 'question', text: 'q' }));
    expect(read('gate')).toBe(false);
    settle(agent('waiting-input'));
    expect(read('gate')).toBe(true);
  });
});
