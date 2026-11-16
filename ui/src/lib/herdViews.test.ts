import { describe, expect, it, vi } from 'vitest';
import { HERD_VIEWS, herdViews, pickView } from './herdViews';
import type { PluginInfo } from './types';

// The views' components import the router, which reads `location` when it
// loads; the tests only look at the registry.
vi.mock('./router.svelte', () => ({
  route: { path: '/', query: new URLSearchParams() },
  href: (p: string) => `#${p}`,
  go: () => {},
}));

const plugin = (name: string, over: Partial<PluginInfo> = {}): PluginInfo => ({
  name,
  kind: 'view',
  version: '1',
  builtin: false,
  settings: [],
  enabled: true,
  login: false,
  view: { entry: 'index.html' },
  ...over,
});

describe('herd views', () => {
  it('have unique ids, the list first', () => {
    const ids = HERD_VIEWS.map((v) => v.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toEqual(['list', 'stages']);
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
  it('list enabled view plugins after the built-ins', () => {
    const plugins = [
      plugin('herd-tiles', { label: 'Tiles', builtin: true }),
      plugin('github', { kind: 'host', view: undefined }),
      plugin('off', { enabled: false }),
      plugin('plain'),
    ];
    const views = herdViews(plugins);
    expect(views.map((v) => v.id)).toEqual(['list', 'stages', 'plugin:herd-tiles', 'plugin:plain']);
    expect(views[2]).toMatchObject({ label: 'Tiles', plugin: 'herd-tiles', entry: 'index.html' });
    expect(views[3].label).toBe('plain');
  });
  it('pick a plugin view, falling back when it is unknown or off', () => {
    const plugins = [plugin('herd-tiles', { label: 'Tiles' }), plugin('off', { enabled: false })];
    expect(pickView('plugin:herd-tiles', plugins).plugin).toBe('herd-tiles');
    expect(pickView('plugin:off', plugins).id).toBe('list');
    expect(pickView('plugin:nope', plugins).id).toBe('list');
    expect(pickView('plugin:herd-tiles').id).toBe('list');
  });
});
