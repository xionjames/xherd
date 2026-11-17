#!/usr/bin/env node
// A mock xherd API for screenshots and videos: serves ui/dist and fake data (generic agents,
// five flows, sixteen jobs in three repositories). No daemon, no tmux, nothing real.
//   pnpm -C ui build && node scripts/demo/server.mjs [port]      (default 4811)
import http from 'node:http'; import fs from 'node:fs'; import path from 'node:path'; import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
import * as C from './content.mjs';

const DIST = path.join(path.dirname(fileURLToPath(import.meta.url)), '../../ui/dist');
const PORT = Number(process.argv[2] ?? 4811);
const now = Date.now(); const ago = (min) => now - min * 60000;

// ---- flows (the stage ids and roles of each) ----
const st = (id, role, label) => ({ id, role, class: 'default', gate: true, ...(label ? { label } : {}) });
const FLOWS = {
  feature: ['spec', 'design', 'tasks', 'implement', 'review', 'ship', 'pr-review', 'pr'].map((i) => st(i, i === 'implement' ? 'implement' : i === 'pr-review' ? 'pr' : i)),
  bugfix: [st('analysis', 'spec'), st('reproduce', 'implement'), st('implement', 'implement', 'Fix'), st('review', 'review'), st('ship', 'ship'), st('pr-review', 'pr'), st('pr', 'pr')],
  refactor: [st('analysis', 'spec'), st('plan', 'tasks'), st('implement', 'implement'), st('review', 'review'), st('ship', 'ship'), st('pr', 'pr')],
  docs: [st('outline', 'spec'), st('write', 'implement', 'Write'), st('review', 'review'), st('publish', 'ship', 'Publish')],
  hotfix: [st('reproduce', 'implement'), st('fix', 'implement', 'Fix'), st('ship', 'ship')],
};
const flows = Object.entries(FLOWS).map(([flow, stages]) => ({ pack: 'xherd', flow, stages }));

// ---- agents (generic personas) ----
const AG = [
  ['planner', 'Turns a ticket into a spec, a design and a task list', 'claude-code', 'opus', 'working', 1, 1, 'WS-142', 212000],
  ['builder', 'Implements one task at a time and proves it with tests', 'claude-code', 'sonnet', 'working', 3, 3, 'WS-155', 538000],
  ['reviewer', 'Reads the diff, files findings, checks the fixes', 'codex', 'gpt-5', 'working', 1, 2, 'BIL-70', 127000],
  ['shipper', 'Writes the pull request and follows it until it merges', 'opencode', '', 'idle', 0, 2, 'WS-138', 41000],
  ['triager', 'Reproduces a bug and finds the root cause before any fix', 'claude-code', 'sonnet', 'working', 1, 1, 'BIL-77', 96000],
  ['docs-writer', 'Keeps the docs and the changelog in step with the code', 'opencode', '', 'working', 1, 1, 'WS-160', 33000],
].map(([name, description, runtime, model, status, liveJobs, maxLiveJobs, lastJob, tokensToday], i) => ({
  id: `ag${i + 1}`, name, description, version: 2 + (i % 3), runtime, models: model ? { [runtime]: model } : {}, status, liveJobs, maxLiveJobs, lastJob, tokensToday, skills: [],
}));
const byName = Object.fromEntries(AG.map((a) => [a.name, a]));
const ASSIGN = {
  feature: ['planner', 'planner', 'planner', 'builder', 'reviewer', 'shipper', 'reviewer', 'shipper'],
  bugfix: ['triager', 'triager', 'builder', 'reviewer', 'shipper', 'reviewer', 'shipper'],
  refactor: ['planner', 'planner', 'builder', 'reviewer', 'shipper', 'shipper'],
  docs: ['planner', 'docs-writer', 'reviewer', 'shipper'],
  hotfix: ['triager', 'builder', 'shipper'],
};

// ---- jobs ----
const REPOS = { 'web-store': '/work/web-store', 'billing-api': '/work/billing-api', 'mobile-app': '/work/mobile-app' };
const T = (id, title, status, extra = {}) => ({ id, title, status, depends: [], files: [], ...extra });
function job(o) {
  const stages = FLOWS[o.flow]; const idx = o.stage ? stages.findIndex((s) => s.id === o.stage) : -1;
  const agents = {}; const stageAgents = {};
  stages.forEach((s, i) => { const a = byName[ASSIGN[o.flow][i]]; agents[a.id] = { id: a.id, name: a.name, version: a.version, runtime: a.runtime }; stageAgents[s.id] = { agent: a.id, human: false }; });
  const cur = idx >= 0 ? byName[ASSIGN[o.flow][idx]] : byName[ASSIGN[o.flow][0]];
  const [ticketKey] = [o.ticket];
  return {
    id: o.name, name: o.name, ticket: { key: ticketKey }, branch: `${ticketKey}-${o.name}`, status: o.status, createdAt: ago(400), statusSince: ago(o.ago), lastActivity: ago(o.last ?? o.ago),
    repo: REPOS[o.repo], workdir: `${REPOS[o.repo]}/worktrees/${o.name}`, base: 'origin/main', pack: 'xherd', flow: o.flow, stage: o.stage,
    visitedStages: idx >= 0 ? stages.slice(0, idx + 1).map((s) => s.id) : [], agents, stageAgents, currentAgent: cur.id,
    runtime: cur.runtime, model: cur.models[cur.runtime], autonomy: 'ask', permissionMode: 'edits', tasks: o.tasks ?? [], commits: o.commits ?? [], ...o.extra,
  };
}
const docsGate = (stage, role, file, extra = {}) => ({ kind: 'docs', stage, role, files: [{ path: file, sha256: 'a'.repeat(64) }], ...extra });
const JOBS = [
  job({ name: 'coupon-codes', ticket: 'WS-142', repo: 'web-store', flow: 'feature', stage: 'design', status: 'waiting-input', ago: 6, extra: { gate: docsGate('design', 'design', '.xherd/spec/design.md', { questions: ['Should a coupon stack with the free-shipping threshold?'] }), lastMessage: 'Design ready for review' } }),
  job({ name: 'order-export-csv', ticket: 'WS-151', repo: 'web-store', flow: 'feature', stage: 'implement', status: 'waiting-input', ago: 14, tasks: [T('T01', 'CSV writer', 'done', { evidence: 'npm test -- export: 6 passed' }), T('T02', 'Export endpoint', 'done'), T('T03', 'Download button', 'todo', { depends: ['T02'] })], commits: [{ id: 'C01', subject: 'Add the CSV writer', files: ['src/export/csv.ts'], status: 'done', tasks: ['T01'], sha: '8c1d2e4' }], extra: { gate: { kind: 'commit', id: 'C02', subject: 'Add CSV export endpoint', message: 'Add CSV export endpoint\n\nGET /orders.csv returns the signed-in customer\'s orders.', files: ['src/export/csv.ts', 'src/routes/orders.ts'], status: 'proposed', tasks: ['T02'] } } }),
  job({ name: 'search-filters', ticket: 'WS-155', repo: 'web-store', flow: 'feature', stage: 'implement', status: 'running', ago: 55, last: 0, tasks: [T('T01', 'Filter model and URL params', 'done', { evidence: 'npm test: 9 passed' }), T('T02', 'Facet counts from the API', 'done'), T('T03', 'Filter chips on the search page', 'doing', { depends: ['T01'] }), T('T04', 'Clear all button', 'todo', { depends: ['T03'] }), T('T05', 'Keyboard support', 'todo', { depends: ['T03'] })] }),
  job({ name: 'cart-rounding', ticket: 'WS-138', repo: 'web-store', flow: 'bugfix', stage: 'pr', status: 'watching', ago: 40, extra: { prUrl: 'https://github.com/acme/web-store/pull/412', prState: 'open', prCi: 'passing', prMergeable: 'yes', prBase: 'main', prChecks: [{ name: 'build', bucket: 'pass', workflow: 'CI' }, { name: 'unit-tests', bucket: 'pass', workflow: 'CI' }, { name: 'lint', bucket: 'pass', workflow: 'CI' }], prReviewers: [{ name: 'maria', state: 'approved' }, { name: 'devon', state: 'pending' }] } }),
  job({ name: 'inventory-sync', ticket: 'WS-171', repo: 'web-store', flow: 'feature', stage: 'pr', status: 'watching', ago: 75, extra: { prUrl: 'https://github.com/acme/web-store/pull/419', prState: 'open', prCi: 'passing', prConflict: true, prMergeable: 'no', prBase: 'main', prChecks: [{ name: 'build', bucket: 'pass', workflow: 'CI' }, { name: 'unit-tests', bucket: 'pass', workflow: 'CI' }], prReviewers: [{ name: 'maria', state: 'commented' }] } }),
  job({ name: 'api-docs-refresh', ticket: 'WS-160', repo: 'web-store', flow: 'docs', stage: 'write', status: 'running', ago: 25, last: 1 }),
  job({ name: 'dark-mode', ticket: 'WS-120', repo: 'web-store', flow: 'feature', stage: 'pr', status: 'done', ago: 300, extra: { prUrl: 'https://github.com/acme/web-store/pull/398', prState: 'merged' } }),
  job({ name: 'invoice-pdf', ticket: 'BIL-81', repo: 'billing-api', flow: 'feature', stage: 'spec', status: 'waiting-input', ago: 21, extra: { gate: docsGate('spec', 'spec', '.xherd/spec/spec.md', { questions: ['Should the PDF carry the company logo, or only text?'] }), lastMessage: 'Spec ready for review' } }),
  job({ name: 'checkout-timeout', ticket: 'BIL-77', repo: 'billing-api', flow: 'bugfix', stage: 'reproduce', status: 'running', ago: 18, last: 0 }),
  job({ name: 'tax-rounding', ticket: 'BIL-70', repo: 'billing-api', flow: 'refactor', stage: 'review', status: 'running', ago: 90, last: 2, tasks: [T('T01', 'Extract a rounding module', 'done'), T('T02', 'Move the callers', 'done'), T('T03', 'Remove the old helpers', 'done')] }),
  job({ name: 'rate-limits', ticket: 'BIL-84', repo: 'billing-api', flow: 'feature', stage: 'tasks', status: 'waiting-input', ago: 33, extra: { gate: docsGate('tasks', 'tasks', '.xherd/spec/tasks.md') } }),
  job({ name: 'sdk-split', ticket: 'BIL-64', repo: 'billing-api', flow: 'refactor', stage: 'analysis', status: 'queued', ago: 4, extra: { currentAgent: 'ag1' } }),
  job({ name: 'webhook-retries', ticket: 'BIL-59', repo: 'billing-api', flow: 'bugfix', stage: 'pr', status: 'done', ago: 1500, extra: { prUrl: 'https://github.com/acme/billing-api/pull/233', prState: 'merged' } }),
  job({ name: 'push-prefs', ticket: 'APP-203', repo: 'mobile-app', flow: 'feature', stage: 'tasks', status: 'running', ago: 12, last: 0 }),
  job({ name: 'login-crash', ticket: 'APP-207', repo: 'mobile-app', flow: 'hotfix', stage: 'fix', status: 'waiting-input', ago: 8, extra: { gate: { kind: 'question', text: 'The crash only happens on Android 9. Should the fix target all versions or only 9?' } } }),
  job({ name: 'offline-sync', ticket: 'APP-198', repo: 'mobile-app', flow: 'feature', stage: 'implement', status: 'paused', ago: 130, tasks: [T('T01', 'Local queue', 'done'), T('T02', 'Retry with backoff', 'doing')] }),
  job({ name: 'onboarding-tour', ticket: 'APP-210', repo: 'mobile-app', flow: 'feature', stage: 'spec', status: 'queued', ago: 2 }),
];
const jobById = Object.fromEntries(JOBS.map((j) => [j.name, j]));

const plugins = [
  { name: 'claude-code', kind: 'runtime', version: '1.0.0', builtin: true, settings: [], enabled: true, login: true },
  { name: 'opencode', kind: 'runtime', version: '1.0.0', builtin: true, settings: [], enabled: true, login: false },
  { name: 'codex', kind: 'runtime', version: '1.0.0', builtin: true, settings: [], enabled: true, login: true },
  { name: 'github', kind: 'host', version: '1.0.0', builtin: true, settings: [], enabled: true, login: true },
];
const state = { version: '0.4.0', buildVersion: '0.4.0', platform: 'linux', config: { maxConcurrentJobs: 12, welcomed: true }, jobs: JOBS, agents: AG, plugins, trackerLinks: [], brokenPlugins: [], flows, sandbox: false };
const todos = [
  { id: 't1', title: 'Check the release branch is green', status: 'due', due: ago(30), repeatMinutes: 0 },
  { id: 't2', title: 'Ask Maria about the invoice template', status: 'open', due: now + 3 * 3600000, repeatMinutes: 0 },
];
const reviews = [
  { jobId: 'tax-rounding', at: ago(70), gate: 'commit C03', files: ['src/tax/round.ts', 'src/tax/round.test.ts'], reason: 'under autonomy important', seen: false },
  { jobId: 'push-prefs', at: ago(120), gate: 'spec', files: ['.xherd/spec/spec.md'], seen: false },
  { jobId: 'checkout-timeout', at: ago(200), gate: 'analysis', files: ['.xherd/spec/spec.md'], seen: false },
];
const skills = [
  { name: 'code-review', description: 'Checklist for reviewing a diff', files: 3, bytes: 4200, usedBy: ['reviewer'] },
  { name: 'write-tests', description: 'How to write a failing test first', files: 2, bytes: 2600, usedBy: ['builder', 'triager'] },
  { name: 'api-docs', description: 'House style for API reference pages', files: 4, bytes: 5100, usedBy: ['docs-writer'] },
  { name: 'db-migrations', description: 'Forward-only schema changes', files: 2, bytes: 2100, usedBy: [] },
];
const sub = (n) => ({ id: n, name: n });
const flowView = (flow) => ({ pack: 'xherd', flow, stages: FLOWS[flow].map((s, i) => ({ id: s.id, role: s.role, class: 'default', gate: true, label: s.label, assigned: true, agent: ASSIGN[flow][i], human: s.role === 'ship' || s.role === 'pr', broken: undefined })) });

const docsOf = (j) => (j.name === 'coupon-codes' ? ['design.md', 'memory.md'] : j.name === 'invoice-pdf' ? ['spec.md', 'memory.md'] : []);
const textOf = (p) => (p.endsWith('design.md') ? C.design : p.endsWith('spec.md') ? C.spec : p.endsWith('memory.md') ? C.memory : '');
const sha = (t) => crypto.createHash('sha256').update(t).digest('hex');
const events = [['stage.enter', { stage: 'implement' }], ['task.start', { task: 'T03' }], ['commit.propose', { id: 'C02' }], ['gate.open', { stage: 'design' }], ['approval', { gate: 'spec', actor: 'user' }], ['stage.enter', { stage: 'spec' }]].map(([kind, payload], i) => ({ id: i + 1, kind, payload, at: ago(5 + i * 9) }));

const routes = { '/state': state, '/checks': { checks: [] }, '/config': { config: state.config, running: state.config, restartNeeded: false }, '/todos': { todos }, '/reviews': { reviews }, '/agents': { agents: AG }, '/skills': { skills }, '/healthz': { ok: true }, '/worklog': { days: [], tickets: [] } };
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.json': 'application/json' };
const json = (r, o) => { r.setHeader('content-type', 'application/json'); r.end(JSON.stringify(o)); };

const srv = http.createServer((q, r) => {
  const url = new URL(q.url, 'http://x'); const u = url.pathname;
  if (u.startsWith('/api/v1/')) {
    const k = u.slice(7); let m;
    if (routes[k]) return json(r, routes[k]);
    if ((m = k.match(/^\/flows\/[^/]+\/([^/]+)$/))) return json(r, flowView(m[1]));
    if ((m = k.match(/^\/jobs\/([^/]+)\/docs$/))) return json(r, { docs: docsOf(jobById[m[1]] ?? {}) });
    if ((m = k.match(/^\/jobs\/([^/]+)\/file$/))) { const p = url.searchParams.get('path') ?? ''; const t = textOf(p); return json(r, { path: p, text: t, sha256: sha(t), reviewed: null }); }
    if ((m = k.match(/^\/jobs\/([^/]+)\/diff$/))) { const j = jobById[m[1]]; const g = j?.gate; const files = g?.kind === 'commit' ? g.files : []; return json(r, { job: m[1], head: 'a1b2c3d', commit: g?.kind === 'commit' ? g : undefined, view: 'commit', files, others: [], diff: C.diff }); }
    if ((m = k.match(/^\/jobs\/([^/]+)$/))) return json(r, { ...(jobById[m[1]] ?? {}), events });
    if (/^\/jobs\/[^/]+\/(viewed|open-targets|findings|memory|agent-memory|ticket|ticket-status)$/.test(k)) return json(r, { viewed: {}, targets: [], findings: [], current: '', proposed: null, text: '' });
    return json(r, {});
  }
  let f = path.join(DIST, u === '/' ? 'index.html' : u);
  if (!fs.existsSync(f) || fs.statSync(f).isDirectory()) f = path.join(DIST, 'index.html');
  r.setHeader('content-type', mime[path.extname(f)] ?? 'application/octet-stream'); r.end(fs.readFileSync(f));
});
// websockets: /ws stays quiet; /ws/term/<job> paints a short fake session.
function frame(buf) { const n = buf.length; const h = n < 126 ? Buffer.from([0x82, n]) : Buffer.from([0x82, 126, n >> 8, n & 255]); return Buffer.concat([h, buf]); }
srv.on('upgrade', (q, s) => {
  const k = crypto.createHash('sha1').update(q.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
  s.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' + k + '\r\n\r\n');
  s.on('error', () => {});
  if (q.url.includes('/ws/term/')) { let b = Buffer.from(C.terminal); while (b.length) { s.write(frame(b.subarray(0, 4000))); b = b.subarray(4000); } }
});
srv.listen(PORT, '127.0.0.1', () => console.log(`demo xherd on http://127.0.0.1:${PORT}/`));
