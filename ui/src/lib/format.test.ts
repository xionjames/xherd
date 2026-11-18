import { describe, expect, it } from 'vitest';
import { needsExpand, needsYou, waitsAtGate } from './format';
import type { Job } from './types';

const a = (over: Partial<Job>): Job => ({ id: 'x', name: 'x', branch: 'x', status: 'running', createdAt: 0, statusSince: 0, lastActivity: 0, ...over });
const docs = { kind: 'docs', stage: 'design', role: 'design', files: [{ path: 'design.md' }] } as unknown as Job['gate'];

describe('waitsAtGate and needsYou (R4)', () => {
  it('an open gate waits for you', () => {
    expect(waitsAtGate(a({ status: 'waiting-input', gate: docs }))).toBe(true);
    expect(needsYou(a({ status: 'waiting-input', gate: docs }))).toBe(true);
  });
  it('an open gate waits for you only once the agent stopped there', () => {
    const running = a({ status: 'running', gate: docs });
    expect(waitsAtGate(running)).toBe(false);
    expect(needsYou(running)).toBe(false);
    for (const status of ['waiting-input', 'idle'] as const) {
      expect(waitsAtGate(a({ status, gate: docs }))).toBe(true);
      expect(needsYou(a({ status, gate: docs }))).toBe(true);
    }
  });
  it('a gate autonomy will approve at the stop does not', () => {
    const x = a({ gate: docs, autoApprovePending: true });
    expect(waitsAtGate(x)).toBe(false);
    expect(needsYou(x)).toBe(false);
  });
  it('no gate, no wait at a gate; blocked and waiting-input still need you', () => {
    expect(waitsAtGate(a({}))).toBe(false);
    expect(needsYou(a({}))).toBe(false);
    expect(needsYou(a({ status: 'blocked', gate: docs, autoApprovePending: true }))).toBe(true);
    expect(needsYou(a({ status: 'waiting-input' }))).toBe(true);
  });
});

describe('needsExpand', () => {
  it('short text fits', () => expect(needsExpand('cargo test: 3 passed')).toBe(false));
  it('long text is cut', () => expect(needsExpand('x'.repeat(121))).toBe(true));
  it('120 characters still fit', () => expect(needsExpand('x'.repeat(120))).toBe(false));
  it('a newline is cut', () => expect(needsExpand('a\nb')).toBe(true));
  it('empty and undefined fit', () => {
    expect(needsExpand('')).toBe(false);
    expect(needsExpand(undefined)).toBe(false);
  });
});
