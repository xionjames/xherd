import { describe, expect, it } from 'vitest';
import { needsYou, waitsAtGate } from './format';
import type { Job } from './types';

const a = (over: Partial<Job>): Job => ({ id: 'x', name: 'x', branch: 'x', status: 'running', createdAt: 0, statusSince: 0, lastActivity: 0, ...over });
const docs = { kind: 'docs', stage: 'design', role: 'design', files: [{ path: 'design.md' }] } as unknown as Job['gate'];

describe('waitsAtGate and needsYou (R4)', () => {
  it('an open gate waits for you', () => {
    expect(waitsAtGate(a({ gate: docs }))).toBe(true);
    expect(needsYou(a({ gate: docs }))).toBe(true);
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
