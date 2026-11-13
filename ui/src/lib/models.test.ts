import { describe, expect, it } from 'vitest';
import { canChooseModel, validModel } from './models';
import type { PluginInfo } from './types';

describe('models', () => {
  it('accepts the names the daemon accepts', () => {
    for (const ok of ['', ' sonnet ', 'opencode/big-pickle', 'claude-opus-5-5[1m]', 'm@2025', 'gpt-5.1:high']) expect(validModel(ok)).toBe(true);
    for (const bad of ['--x', 'a b', 'x;y', '$(x)', 'm'.repeat(101)]) expect(validModel(bad)).toBe(false);
  });
  it('knows which runtimes can choose a model', () => {
    const p = (name: string, kind: string, chooseModel?: boolean) =>
      ({ name, kind, chooseModel, version: '1', builtin: true, settings: [], enabled: true, login: false }) as PluginInfo;
    const plugins = [p('claude-code', 'runtime', true), p('old', 'runtime', false), p('github', 'host', true)];
    expect(canChooseModel(plugins, 'claude-code')).toBe(true);
    expect(canChooseModel(plugins, 'old')).toBe(false);
    expect(canChooseModel(plugins, 'github')).toBe(false);
    expect(canChooseModel(plugins, 'missing')).toBe(false);
  });
});
