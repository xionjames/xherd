import { describe, expect, it, vi } from 'vitest';
vi.mock('./store.svelte', () => ({ jobByName: () => undefined }));
vi.mock('./router.svelte', () => ({ go: () => {} }));
import { addNotice, BLOCK_TEXT, browserBlock, notices, settle } from './notices.svelte';
import type { Job } from './types';

const agent = (status: Job['status'], gate?: Job['gate']) =>
  ({ id: 'A1', name: 'a', status, gate }) as Job;
const read = (kind: string) => notices.list.find((n) => n.jobId === 'A1' && n.kind === kind)!.read;

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
    expect(notices.list.find((n) => n.jobId === 'B2')!.read).toBe(false);
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

describe('browserBlock', () => {
  const env = (secure: boolean, notification: boolean, permission?: NotificationPermission) =>
    ({ secure, notification, permission });

  it('says why browser notifications cannot be on, the insecure page first', () => {
    expect(browserBlock(env(false, false))).toBe('insecure');
    expect(browserBlock(env(false, true, 'denied'))).toBe('insecure');
    expect(browserBlock(env(true, false))).toBe('unsupported');
    expect(browserBlock(env(true, true, 'denied'))).toBe('denied');
  });

  it('is null on a secure page that may still ask or already may', () => {
    expect(browserBlock(env(true, true, 'default'))).toBeNull();
    expect(browserBlock(env(true, true, 'granted'))).toBeNull();
  });

  it('has a sentence for each reason, the insecure one naming localhost', () => {
    expect(BLOCK_TEXT.insecure).toContain('127.0.0.1');
    expect(BLOCK_TEXT.denied).toContain('blocked');
    expect(BLOCK_TEXT.unsupported).toContain('does not support');
  });
});
