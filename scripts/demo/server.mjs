#!/usr/bin/env node
// A mock xherd API for screenshots and videos: serves ui/dist and fake data (generic agents,
// five flows, sixteen jobs in three repositories). No daemon, no tmux, nothing real.
//   pnpm -C ui build && node scripts/demo/server.mjs [port]      (default 4811)
import http from 'node:http'; import fs from 'node:fs'; import path from 'node:path'; import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url'; import zlib from 'node:zlib';
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
  job({ name: 'search-filters', ticket: 'WS-155', repo: 'web-store', flow: 'feature', stage: 'implement', status: 'running', ago: 55, last: 0, tasks: [T('T01', 'Filter model and URL params', 'done', { evidence: 'npm test: 9 passed' }), T('T02', 'Facet counts from the API', 'done'), T('T03', 'Filter chips on the search page', 'doing', { depends: ['T01'] }), T('T04', 'Clear all button', 'todo', { depends: ['T03'] }), T('T05', 'Keyboard support', 'todo', { depends: ['T03'] })], commits: [{ id: 'C01', subject: 'Add the filter model', files: ['src/search/filters.ts'], status: 'done', tasks: ['T01'], sha: '5e7a1b9', pushed: false }] }),
  job({ name: 'cart-rounding', ticket: 'WS-138', repo: 'web-store', flow: 'bugfix', stage: 'pr', status: 'watching', ago: 40, commits: [{ id: 'C01', subject: 'Round the cart total once', files: ['src/cart/total.ts'], status: 'done', tasks: ['T01'], sha: '2b4c6d8', pushed: false }], extra: { prUrl: 'https://github.com/acme/web-store/pull/412', prState: 'open', prCi: 'passing', prMergeable: 'yes', prBase: 'main', prChecks: [{ name: 'build', bucket: 'pass', workflow: 'CI' }, { name: 'unit-tests', bucket: 'pass', workflow: 'CI' }, { name: 'lint', bucket: 'pass', workflow: 'CI' }], prReviewers: [{ name: 'maria', state: 'approved' }, { name: 'devon', state: 'pending' }] } }),
  job({ name: 'inventory-sync', ticket: 'WS-171', repo: 'web-store', flow: 'feature', stage: 'pr', status: 'watching', ago: 75, commits: [{ id: 'C01', subject: 'Sync stock levels nightly', files: ['src/inventory/sync.ts'], status: 'done', tasks: ['T01'], sha: '9f3e1a2', pushed: true }], extra: { prUrl: 'https://github.com/acme/web-store/pull/419', prState: 'open', prCi: 'passing', prConflict: true, prMergeable: 'no', prBase: 'main', prChecks: [{ name: 'build', bucket: 'pass', workflow: 'CI' }, { name: 'unit-tests', bucket: 'pass', workflow: 'CI' }], prReviewers: [{ name: 'maria', state: 'commented' }] } }),
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

// ---- actions: services and tasks a job runs next to the agent ----
const act = (id, label, kind, run, o = {}) => ({ def: { id, label, description: o.description ?? '', run, cwd: o.cwd ?? '', kind, icon: '' }, source: o.source ?? 'job', ...(o.approved === false ? {} : { approved: 'a'.repeat(64) }), ...(o.last ? { last: o.last } : {}), succeeded: o.succeeded ?? false });
const running = { startedAt: ago(20), state: 'running' };
const okRun = { startedAt: ago(45), endedAt: ago(44), state: 'exited', exit: 0 };
const ACTIONS = {
  'search-filters': [
    act('dev', 'Dev server', 'service', 'npm run dev -- --port $PORT_BASE', { description: 'Vite dev server for the search page', source: 'profile', last: running }),
    act('storybook', 'Storybook', 'service', 'npm run storybook', { source: 'profile' }),
    act('build', 'Build', 'task', 'npm run build', { last: okRun, succeeded: true }),
    act('seed', 'Seed the database', 'task', 'node scripts/seed.js --reset', { approved: false }),
  ],
  'dark-mode': [
    act('build', 'Build', 'task', 'npm run build', { last: okRun, succeeded: true }),
    act('e2e', 'End-to-end tests', 'task', 'npx playwright test', { last: { startedAt: ago(60), endedAt: ago(52), state: 'exited', exit: 0 }, succeeded: true, source: 'profile' }),
    act('preview', 'Preview', 'service', 'npm run preview', { last: { startedAt: ago(70), endedAt: ago(65), state: 'stopped' } }),
  ],
};
for (const [n, a] of Object.entries(ACTIONS)) jobById[n].actions = a;
jobById['search-filters'].portBase = 4400;
const actionsView = (n) => { const a = ACTIONS[n] ?? []; return { actions: a, portBase: n === 'search-filters' ? 4400 : undefined, saveOffer: a.filter((x) => x.succeeded && x.source === 'job').map((x) => x.def.id), hasProfile: true }; };

const plugins = [
  { name: 'claude-code', kind: 'runtime', version: '1.0.0', builtin: true, settings: [], enabled: true, login: true },
  { name: 'opencode', kind: 'runtime', version: '1.0.0', builtin: true, settings: [], enabled: true, login: false },
  { name: 'codex', kind: 'runtime', version: '1.0.0', builtin: true, settings: [], enabled: true, login: true },
  { name: 'github', kind: 'host', version: '1.0.0', builtin: true, settings: [], enabled: true, login: true },
  { name: 'gitea', kind: 'host', version: '0.2.0', builtin: false, settings: [], enabled: true, login: false, source: { url: 'https://github.com/acme/xherd-gitea', ref: null, path: null, sha: '3f2a9c1d0b7e4a5c8d9e1f2a3b4c5d6e7f8a9b0c', digest: 'a'.repeat(64), installedAt: '2026-11-01T10:00:00Z' } },
];
const pluginsList = () => ({ plugins: plugins.map(({ name, kind, version, builtin, source }) => ({ name, kind, version, builtin, source })), restartNeeded: false, installEnabled: true });
const pluginPreview = (url) => ({ url, ref: null, path: null, sha: '9d8c7b6a5f4e3d2c1b0a9f8e7d6c5b4a3f2e1d0c', name: 'forgejo', kind: 'host', version: '0.1.0', label: 'Forgejo', description: 'Pull requests on a Forgejo server', compatible: true, digest: 'b'.repeat(64), files: 6, bytes: 5400, replaces: null, settings: [{ key: 'token', label: 'Access token', secret: true, required: true }], runs: [{ what: 'pr.create', argv: ['./bin/forgejo', 'pr', 'create'] }], warning: 'A plugin runs commands as you, with your environment. Install only code you trust.' });
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
// Image fixtures for the by-hand check of the image preview: a 1x1 PNG per colour and an SVG, each with its blob id.
const png = (rgb) => { const crc = (b) => { let c, x = ~0; for (const v of b) { c = (x ^ v) & 255; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; x = (x >>> 8) ^ c; } return ~x >>> 0; };
  const ch = (t, d) => { const b = Buffer.concat([Buffer.from(t), d]); const o = Buffer.alloc(4), c = Buffer.alloc(4); o.writeUInt32BE(d.length); c.writeUInt32BE(crc(b)); return Buffer.concat([o, b, c]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(1, 0); ihdr.writeUInt32BE(1, 4); ihdr[8] = 8; ihdr[9] = 2;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), ch('IHDR', ihdr), ch('IDAT', zlib.deflateSync(Buffer.from([0, ...rgb]))), ch('IEND', Buffer.alloc(0))]); };
const IMAGES = {
  'docs/img/logo.png': { type: 'image/png', old: png([30, 90, 200]), new: png([240, 120, 30]) },
  'docs/img/chart.png': { type: 'image/png', new: png([40, 160, 90]) },
  'docs/img/banner.png': { type: 'image/png', old: png([120, 120, 120]), new: png([120, 120, 120]) },
  'docs/img/mark.svg': { type: 'image/svg+xml', new: Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><circle cx="32" cy="32" r="28" fill="#1e5ac8"/></svg>') },
};
const blobOf = (b) => crypto.createHash('sha1').update(b).digest('hex').slice(0, 7);
const imageOf = (p, side) => { const i = IMAGES[p.replace(/^\.\//, '')]; return i && (i[side] ?? (side === 'old' ? undefined : i.new)); };
const imageDiff = () => {
  const l = IMAGES['docs/img/logo.png'], c = IMAGES['docs/img/chart.png'], b = IMAGES['docs/img/banner.png'];
  return `diff --git a/docs/img/logo.png b/docs/img/logo.png
index ${blobOf(l.old)}..${blobOf(l.new)} 100644
Binary files a/docs/img/logo.png and b/docs/img/logo.png differ
diff --git a/docs/img/chart.png b/docs/img/chart.png
new file mode 100644
index 0000000..${blobOf(c.new)}
Binary files /dev/null and b/docs/img/chart.png differ
diff --git a/docs/img/old-banner.png b/docs/img/banner.png
similarity index 100%
rename from docs/img/old-banner.png
rename to docs/img/banner.png
index ${blobOf(b.old)}..${blobOf(b.new)}
diff --git a/docs/README.md b/docs/README.md
new file mode 100644
index 0000000..1a2b3c4
--- /dev/null
+++ b/docs/README.md
@@ -0,0 +1,5 @@
+# Export
+
+![Local logo](img/logo.png)
+
+![Remote image](https://example.com/remote.png)
`;
};
const README = '# Export\n\n![Local logo](img/logo.png)\n\n![Remote image](https://example.com/remote.png)\n';
const events = [['stage.enter', { stage: 'implement' }], ['task.start', { task: 'T03' }], ['commit.propose', { id: 'C02' }], ['gate.open', { stage: 'design' }], ['approval', { gate: 'spec', actor: 'user' }], ['stage.enter', { stage: 'spec' }]].map(([kind, payload], i) => ({ id: i + 1, kind, payload, at: ago(5 + i * 9) }));

let packResolved = false;
const packFiles = [{ runtime: 'claude-code', path: '~/.claude/commands/xherd/design.md', state: 'edited', edited: true }, { runtime: 'claude-code', path: '~/.claude/commands/xherd/old-notes.md', state: 'obsolete', edited: true }];
const installingCheck = { id: 'plugin:opencode:requires:opencode', group: 'plugins', label: 'opencode · requires:opencode', required: false, ok: false, detail: 'Installing opencode 1.19.0…', installing: true };
const packChecks = () => ({ checks: [installingCheck, ...(packResolved ? [] : [{ id: 'pack-files:claude-code', group: 'runtime', label: 'Pack files (claude-code)', required: true, ok: false, detail: '2 files need a decision', fix: { kind: 'review', label: 'Review files' } }])] });
const routes = { '/state': state, '/config': { config: state.config, running: state.config, restartNeeded: false }, '/todos': { todos }, '/reviews': { reviews }, '/agents': { agents: AG }, '/skills': { skills }, '/healthz': { ok: true }, '/worklog': { days: [], tickets: [] } };
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.json': 'application/json' };
const json = (r, o) => { r.setHeader('content-type', 'application/json'); r.end(JSON.stringify(o)); };

const srv = http.createServer((q, r) => {
  const url = new URL(q.url, 'http://x'); const u = url.pathname;
  if (u.startsWith('/api/v1/')) {
    const k = u.slice(7); let m;
    if (k === '/checks') return json(r, packChecks());
    if (k === '/install/review') return json(r, { files: packResolved ? [] : packFiles });
    if (k === '/install/resolve') { packResolved = true; return json(r, packChecks()); }
    if (routes[k]) return json(r, routes[k]);
    if (k === '/plugins') return json(r, pluginsList());
    if (k === '/plugins/preview') return json(r, pluginPreview('https://github.com/acme/xherd-forgejo'));
    if (k === '/plugins/install') return json(r, { name: 'forgejo', kind: 'host', version: '0.1.0', url: 'https://github.com/acme/xherd-forgejo', sha: '9d8c7b6a5f4e3d2c1b0a9f8e7d6c5b4a3f2e1d0c', digest: 'b'.repeat(64) });
    if (k === '/admin/restart') return json(r, { restarting: true });
    if ((m = k.match(/^\/plugins\/([^/]+)\/update$/))) return json(r, { ...pluginPreview('https://github.com/acme/xherd-' + m[1]), name: m[1], installed: { url: 'https://github.com/acme/xherd-' + m[1], sha: '3f2a9c1d0b7e4a5c8d9e1f2a3b4c5d6e7f8a9b0c', version: '0.2.0' }, manifestDiff: '--- a/xherd-plugin.json\n+++ b/xherd-plugin.json\n@@ -2,3 +2,3 @@\n-  "version": "0.2.0",\n+  "version": "0.3.0",\n' });
    if ((m = k.match(/^\/plugins\/([^/]+)$/)) && q.method === 'DELETE') return json(r, { name: m[1], forgot: false });
    if ((m = k.match(/^\/flows\/[^/]+\/([^/]+)$/))) return json(r, flowView(m[1]));
    if ((m = k.match(/^\/jobs\/([^/]+)\/docs$/))) return json(r, { docs: docsOf(jobById[m[1]] ?? {}) });
    if ((m = k.match(/^\/jobs\/([^/]+)\/file$/))) { const p = url.searchParams.get('path') ?? ''; const t = textOf(p); return json(r, { path: p, text: t, sha256: sha(t), reviewed: null }); }
    if ((m = k.match(/^\/jobs\/([^/]+)\/diff$/))) { const j = jobById[m[1]]; const g = j?.gate; const files = g?.kind === 'commit' ? g.files : []; return json(r, { job: m[1], head: 'a1b2c3d', commit: g?.kind === 'commit' ? g : undefined, view: 'commit', files: files.length ? [...files, 'docs/img/logo.png', 'docs/img/chart.png', 'docs/img/banner.png', 'docs/README.md'] : files, others: [], diff: files.length ? C.diff + imageDiff() : C.diff }); }
    if ((m = k.match(/^\/jobs\/([^/]+)\/diff\/source$/))) { const p = url.searchParams.get('path') ?? ''; return json(r, { path: p, rev: 'a1b2c3d', text: p.endsWith('README.md') ? README : '' }); }
    if ((m = k.match(/^\/jobs\/([^/]+)\/image$/))) { const p = url.searchParams.get('path') ?? ''; const side = url.searchParams.get('side') === 'old' ? 'old' : 'new'; const b = imageOf(p, side); if (!b) { r.statusCode = 404; return json(r, { error: { code: 'not_found', message: 'no such image' } }); } r.setHeader('content-type', IMAGES[p.replace(/^\.\//, '')].type); r.setHeader('X-Xherd-Blob', blobOf(b)); return r.end(b); }
    if ((m = k.match(/^\/jobs\/([^/]+)\/actions$/))) return json(r, actionsView(m[1]));
    if ((m = k.match(/^\/jobs\/([^/]+)\/push$/)) && q.method === 'POST') return json(r, { job: { ...(jobById[m[1]] ?? {}), events } });
    if ((m = k.match(/^\/jobs\/([^/]+)\/actions\/([^/]+)\/log$/))) return json(r, { log: C.actionLog, last: (ACTIONS[m[1]] ?? []).find((x) => x.def.id === m[2])?.last ?? null });
    if ((m = k.match(/^\/jobs\/([^/]+)\/actions\//)) && q.method !== 'GET') return json(r, { job: { ...(jobById[m[1]] ?? {}), events } });
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
