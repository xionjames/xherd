import { describe, expect, it, vi } from 'vitest';
const fake = vi.hoisted(() => {
  // The router reads the location and listens for hash changes when loaded.
  Object.assign(globalThis, { location: { hash: "" }, addEventListener: () => {} });
  return { app: { state: { jobs: [] as unknown[] } } };
});
vi.mock('./store.svelte', () => fake);
vi.mock('./api', () => ({ get: async () => ({}) }));
import { approvalHref, attention, attentionCount, type AutoApproval } from './attention.svelte';

const item = (gate: string, files: string[] = []): AutoApproval => ({ jobId: 'a', at: 1, gate, files, seen: false });

describe('attentionCount (R1)', () => {
  it('counts waiting agents and due or failed TODOs, never approvals', () => {
    fake.app.state.jobs = [
      { id: 'a', name: 'a', status: 'waiting-input', statusSince: 0, gate: { kind: 'docs', stage: 'design', files: [] } },
      { id: 'b', name: 'b', status: 'running', statusSince: 0 },
      { id: 'c', name: 'c', status: 'running', statusSince: 0, gate: { kind: 'docs', stage: 'design', files: [] }, autoApprovePending: true },
    ];
    attention.todos = [
      { id: '1', title: 'x', due: 0, status: 'due' },
      { id: '2', title: 'y', due: 0, status: 'error' },
      { id: '3', title: 'z', due: 0, status: 'open' },
    ];
    attention.reviews = [item('design', ['design.md']), item('commit C01')];
    expect(attentionCount()).toBe(3);
  });
  it('is zero when only approvals are unseen', () => {
    fake.app.state.jobs = [];
    attention.todos = [];
    attention.reviews = [item('design', ['design.md'])];
    expect(attentionCount()).toBe(0);
  });
});

describe('approvalHref (R3)', () => {
  it('a docs gate opens the Spec tab on its first document', () => {
    expect(approvalHref(item('design', ['design.md', 'notes.md']), 'cache')).toBe('#/jobs/cache?s=spec&doc=design.md');
  });
  it('a commit gate opens the Changes tab on that commit', () => {
    expect(approvalHref(item('commit C03', ['./src/a.rs']), 'cache')).toBe('#/jobs/cache?s=changes&view=commit&commit=C03');
  });
  it('a docs gate without files opens the Spec tab', () => {
    expect(approvalHref(item('spec'), 'cache')).toBe('#/jobs/cache?s=spec');
  });
});
