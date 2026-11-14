import { describe, expect, it, vi } from 'vitest';
import { HERD_VIEWS, pickView } from './herdViews';

// The views' components import the router, which reads `location` when it
// loads; the tests only look at the registry.
vi.mock('./router.svelte', () => ({
  route: { path: '/', query: new URLSearchParams() },
  href: (p: string) => `#${p}`,
  go: () => {},
}));

describe('herd views', () => {
  it('have unique ids, the list first', () => {
    const ids = HERD_VIEWS.map((v) => v.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids[0]).toBe('list');
    expect(ids).toContain('stages');
    expect(ids).toContain('tiles');
  });
  it('pick a view by id', () => {
    expect(pickView('stages').id).toBe('stages');
    expect(pickView('list').id).toBe('list');
  });
  it('fall back to the first view for an unknown or missing id', () => {
    expect(pickView('nope')).toBe(HERD_VIEWS[0]);
    expect(pickView(null)).toBe(HERD_VIEWS[0]);
    expect(pickView(undefined)).toBe(HERD_VIEWS[0]);
  });
});
