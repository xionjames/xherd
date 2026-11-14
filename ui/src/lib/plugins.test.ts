import { describe, expect, it } from 'vitest';
import { lastOn, matches, needLogin } from './plugins';
import type { Check, PluginInfo } from './types';

const plugin = (name: string, kind: string, extra: Partial<PluginInfo> = {}): PluginInfo => ({
  name,
  kind,
  version: '1.0.0',
  builtin: true,
  settings: [],
  enabled: true,
  login: false,
  ...extra,
});

const all = [
  plugin('claude-code', 'runtime', { label: 'Claude Code', description: 'Anthropic coding agent', login: true }),
  plugin('opencode', 'runtime', { enabled: false }),
  plugin('github', 'host', { label: 'GitHub', description: 'Pull requests through gh', login: true }),
  plugin('gitlab', 'host', { label: 'GitLab', enabled: false }),
  plugin('bitbucket', 'host', { label: 'Bitbucket', description: 'Git access for Bitbucket', enabled: false }),
  plugin('github-issues', 'tracker', { label: 'GitHub Issues' }),
  plugin('jira', 'tracker', { label: 'Jira', description: 'Links to Jira tickets', enabled: false }),
  plugin('uv', 'toolchain', { label: 'Python (uv)' }),
];

const names = (q: string) => all.filter((p) => matches(p, q)).map((p) => p.name);

describe('plugin search', () => {
  it('matches name, label and description, case-insensitive', () => {
    expect(names('git')).toEqual(['github', 'gitlab', 'bitbucket', 'github-issues']);
    expect(names('GIT')).toEqual(names('git'));
    expect(names('anthropic')).toEqual(['claude-code']);
  });

  it('matches the kind and its label', () => {
    expect(names('languages')).toEqual(['uv']);
    expect(names('toolchain')).toEqual(['uv']);
    expect(names('git provider')).toEqual(['github', 'gitlab', 'bitbucket']);
  });

  it('an empty query matches everything, nonsense nothing', () => {
    expect(names('')).toHaveLength(all.length);
    expect(names('  ')).toHaveLength(all.length);
    expect(names('ZZZ')).toEqual([]);
  });
});

describe('last plugin on', () => {
  it('the only runtime on cannot be switched off', () => {
    expect(lastOn(all[0], all)).toBe(true);
    expect(lastOn(all[0], all.map((p) => (p.name === 'opencode' ? { ...p, enabled: true } : p)))).toBe(false);
  });

  it('Git providers and issue trackers are optional', () => {
    expect(lastOn(all[2], all)).toBe(false);
    expect(lastOn(all[5], all)).toBe(false);
  });

  it('a plugin that is off is never the last one on', () => {
    expect(lastOn(all[1], all)).toBe(false);
  });
});

describe('plugins that need a login', () => {
  const check = (name: string, ok: boolean): Check => ({ id: `plugin:${name}:login`, group: 'plugins', label: 'login', required: false, ok, detail: '' });

  it('lists enabled plugins whose login check fails', () => {
    expect(needLogin(all, [check('claude-code', true), check('github', false)]).map((p) => p.name)).toEqual(['github']);
  });

  it('ignores plugins that are off, have no login, or no check yet', () => {
    const off = all.map((p) => (p.name === 'github' ? { ...p, enabled: false } : p));
    expect(needLogin(off, [check('github', false)])).toEqual([]);
    expect(needLogin(all, [check('uv', false)])).toEqual([]);
    expect(needLogin(all, [])).toEqual([]);
    expect(needLogin(all, null)).toEqual([]);
  });
});
