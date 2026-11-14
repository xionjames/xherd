import { describe, expect, it } from 'vitest';
import { acceptMessage, dataMessage, project } from './viewPlugin';
import type { Agent, FlowInfo } from './types';

const flows: FlowInfo[] = [
  {
    pack: 'xherd',
    flow: 'feature',
    stages: [
      { id: 'spec', role: 'spec', class: 'important', gate: true },
      { id: 'implement', role: 'implement', class: 'routine', gate: true },
    ],
  },
];

const agent = (extra: Partial<Agent> = {}): Agent => ({
  id: 'a1',
  name: 'cache',
  branch: 'PAY-1-cache',
  status: 'waiting-input',
  createdAt: 1,
  statusSince: 2,
  lastActivity: 3,
  workdir: '/home/me/xherd/worktrees/app/cache',
  repo: '/home/me/src/app',
  base: 'origin/main',
  runtime: 'claude-code',
  pack: 'xherd',
  flow: 'feature',
  stage: 'spec',
  ticket: { key: 'PAY-1', url: 'https://example.com/PAY-1' },
  gate: {
    kind: 'docs',
    stage: 'spec',
    role: 'spec',
    files: [{ path: 'spec.md', sha256: 'abc' }],
    needsHuman: 'secret: sk-…',
    questions: ['Which database?'],
    asked: ['Is Redis fine?'],
  },
  prUrl: 'https://github.com/acme/app/pull/1',
  prState: 'open',
  prCi: 'passing',
  lastMessage: 'I wrote the spec',
  nextPrompt: '/xherd:design',
  blockReason: 'git access',
  memoryProposal: { file: 'AGENTS.proposed.md', reason: 'new rule', at: 4 },
  ...extra,
});

describe('project', () => {
  it('keeps the listed fields', () => {
    const v = project(agent(), flows);
    expect(v).toEqual({
      id: 'a1',
      name: 'cache',
      status: 'waiting-input',
      statusSince: 2,
      createdAt: 1,
      lastActivity: 3,
      repo: 'app',
      branch: 'PAY-1-cache',
      ticket: { key: 'PAY-1', url: 'https://example.com/PAY-1' },
      runtime: 'claude-code',
      flow: 'feature',
      stage: 'spec',
      stageName: 'Spec',
      waitsOn: 'spec.md',
      gate: 'docs',
      needsYou: true,
      hue: expect.stringMatching(/^hsl\(/),
      prUrl: 'https://github.com/acme/app/pull/1',
      prState: 'open',
      prCi: 'passing',
    });
  });

  it('leaves out messages, prompts, gate texts and paths', () => {
    const text = JSON.stringify(project(agent(), flows));
    for (const s of ['I wrote the spec', '/xherd:design', 'Which database', 'Redis', 'sk-', 'abc', '/home/me', 'origin/main', 'git access', 'AGENTS.proposed'])
      expect(text).not.toContain(s);
    const q = project(agent({ gate: { kind: 'question', text: 'Secret question?' } }), flows);
    expect(JSON.stringify(q)).not.toContain('Secret question');
    expect(q.gate).toBe('question');
  });

  it('carries only the key and url of a ticket', () => {
    const odd = { key: 'K', url: 'https://x', extra: 'leak' } as unknown as Agent['ticket'];
    expect(project(agent({ ticket: odd }), flows).ticket).toEqual({ key: 'K', url: 'https://x' });
  });

  it('works without a stage, gate or repo', () => {
    const v = project(agent({ stage: undefined, gate: undefined, repo: undefined, status: 'running', ticket: undefined }), flows);
    expect(v.stage).toBeUndefined();
    expect(v.stageName).toBeUndefined();
    expect(v.waitsOn).toBeUndefined();
    expect(v.gate).toBeUndefined();
    expect(v.ticket).toBeUndefined();
    expect(v.needsYou).toBe(false);
    expect(v.repo).toBe('folder');
  });
});

describe('dataMessage', () => {
  it('is protocol v1 with projected agents', () => {
    const m = dataMessage([agent()], flows, 'dark');
    expect(m.xherdView).toBe(1);
    expect(m.kind).toBe('data');
    expect(m.theme).toBe('dark');
    expect(m.flows).toEqual(flows);
    expect(m.agents).toEqual([project(agent(), flows)]);
    expect(JSON.stringify(m)).not.toContain('I wrote the spec');
  });

  it('is plain data that survives structured cloning', () => {
    const m = dataMessage([agent()], flows, 'light');
    expect(structuredClone(m)).toEqual(m);
  });
});

describe('acceptMessage', () => {
  const frame = { name: 'frame' };
  const ids = new Set(['a1']);
  const ev = (data: unknown, extra: Partial<{ source: unknown; origin: string }> = {}) => ({ source: frame, origin: 'null', data, ...extra });

  it('takes ready and open from the frame', () => {
    expect(acceptMessage(ev({ xherdView: 1, kind: 'ready' }), frame, ids)).toEqual({ kind: 'ready' });
    expect(acceptMessage(ev({ xherdView: 1, kind: 'open', agentId: 'a1' }), frame, ids)).toEqual({ kind: 'open', id: 'a1' });
  });

  it('refuses another source', () => {
    expect(acceptMessage(ev({ xherdView: 1, kind: 'ready' }, { source: {} }), frame, ids)).toBeNull();
    expect(acceptMessage(ev({ xherdView: 1, kind: 'ready' }, { source: null }), null, ids)).toBeNull();
  });

  it('refuses another origin', () => {
    expect(acceptMessage(ev({ xherdView: 1, kind: 'ready' }, { origin: 'http://127.0.0.1:4740' }), frame, ids)).toBeNull();
    expect(acceptMessage(ev({ xherdView: 1, kind: 'ready' }, { origin: '' }), frame, ids)).toBeNull();
  });

  it('refuses another version or shape', () => {
    expect(acceptMessage(ev({ xherdView: 2, kind: 'ready' }), frame, ids)).toBeNull();
    expect(acceptMessage(ev({ xherdView: '1', kind: 'ready' }), frame, ids)).toBeNull();
    expect(acceptMessage(ev({ kind: 'ready' }), frame, ids)).toBeNull();
    expect(acceptMessage(ev('ready'), frame, ids)).toBeNull();
    expect(acceptMessage(ev(null), frame, ids)).toBeNull();
  });

  it('refuses an unknown kind or id', () => {
    expect(acceptMessage(ev({ xherdView: 1, kind: 'data' }), frame, ids)).toBeNull();
    expect(acceptMessage(ev({ xherdView: 1, kind: 'navigate', to: '#/settings' }), frame, ids)).toBeNull();
    expect(acceptMessage(ev({ xherdView: 1, kind: 'open', agentId: 'other' }), frame, ids)).toBeNull();
    expect(acceptMessage(ev({ xherdView: 1, kind: 'open' }), frame, ids)).toBeNull();
    expect(acceptMessage(ev({ xherdView: 1, kind: 'open', agentId: ['a1'] }), frame, ids)).toBeNull();
  });
});
