import { describe, expect, it } from 'vitest';
import { runtimeModels, validModel } from './models';
import type { PluginInfo } from './types';

describe('models', () => {
  it('accepts the names the daemon accepts', () => {
    for (const ok of ['', ' sonnet ', 'opencode/big-pickle', 'claude-opus-5-5[1m]', 'm@2025', 'gpt-5.1:high']) expect(validModel(ok)).toBe(true);
    for (const bad of ['--x', 'a b', 'x;y', '$(x)', 'm'.repeat(101)]) expect(validModel(bad)).toBe(false);
  });
  it('knows which runtimes can choose a model', () => {
    const p = (name: string, kind: string, models?: PluginInfo['models']) =>
      ({ name, kind, models, version: '1', builtin: true, settings: [], enabled: true, login: false }) as PluginInfo;
    const plugins = [p('claude-code', 'runtime', [{ id: 'opus' }]), p('old', 'runtime', null), p('github', 'host')];
    expect(runtimeModels(plugins, 'claude-code')).toEqual([{ id: 'opus' }]);
    expect(runtimeModels(plugins, 'old')).toBeNull();
    expect(runtimeModels(plugins, 'github')).toBeNull();
  });
});
