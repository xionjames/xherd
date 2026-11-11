<script lang="ts">
  // `diff` panel: what the user reviews before approving (GET /agents/:id/diff):
  // the proposed commit, the branch at a ship gate, else every uncommitted change.
  import type { Agent, DiffAnswer } from '../lib/types';
  import { get, post } from '../lib/api';
  import { mdNoteText, type MdNote } from '../lib/notes';
  import { showError } from '../lib/errors.svelte';
  import { route, go } from '../lib/router.svelte';
  import { parseUnified } from '../lib/diff';
  import DiffLines, { lineKey } from '../components/DiffLines.svelte';
  import FilePreview from '../components/FilePreview.svelte';
  import Seg from '../components/Seg.svelte';

  let { agent }: { agent: Agent } = $props();
  // Effects key on the id: the agent object is replaced on every update.
  const id = $derived(agent.id);
  let d = $state<DiffAnswer | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  // Inline review comments (#25): kept here until sent as one change request.
  let notes = $state<Record<string, Record<string, string>>>({});
  let sending = $state(false);
  const setNote = (path: string) => (key: string, text: string | null) => {
    const n = { ...(notes[path] ?? {}) };
    if (text) n[key] = text; else delete n[key];
    notes[path] = n;
  };
  let mdNotes = $state<Record<string, Record<string, MdNote>>>({});
  const setMd = (path: string) => (key: string, text: string | null, quote: string) => {
    const n = { ...(mdNotes[path] ?? {}) };
    if (text) n[key] = { text, quote }; else delete n[key];
    mdNotes[path] = n;
  };
  const count = $derived(
    Object.values(notes).reduce((a, n) => a + Object.keys(n).length, 0) +
    Object.values(mdNotes).reduce((a, n) => a + Object.keys(n).length, 0),
  );
  async function sendNotes() {
    const out: string[] = [];
    for (const f of files) {
      for (const l of f.lines) {
        const t = notes[f.path]?.[lineKey(l)];
        if (t) out.push(`${f.path}:${l.new ?? l.old}${l.kind === 'd' ? ' (removed line)' : ''}\n> ${l.text}\n${t}`);
      }
    }
    for (const [path, n] of Object.entries(mdNotes)) out.push(...mdNoteText(path, n));
    sending = true;
    try {
      await post(`/agents/${id}/request-changes`, { comment: `Review comments on the diff:\n\n${out.join('\n\n')}` });
      notes = {}; mdNotes = {};
    } catch (e) { showError('Could not send the comments', e); }
    finally { sending = false; }
  }
  // "Viewed" marks (spec §16): a file's mark holds while its diff is unchanged.
  let viewed = $state<Record<string, string>>({});
  const fingerprint = (f: { lines: { kind: string; text: string }[] }) => {
    let h = 5381;
    for (const l of f.lines) for (const c of l.kind + l.text + '\n') h = ((h * 33) ^ c.charCodeAt(0)) >>> 0;
    return String(h);
  };
  const isViewed = (f: { path: string; lines: { kind: string; text: string }[] }) => viewed[f.path] === fingerprint(f);
  async function toggleViewed(f: { path: string; lines: { kind: string; text: string }[] }) {
    const fp = isViewed(f) ? undefined : fingerprint(f);
    try {
      await post(`/agents/${id}/viewed`, { path: f.path, ...(fp ? { fingerprint: fp } : {}) });
      const v = { ...viewed };
      if (fp) v[f.path] = fp; else delete v[f.path];
      viewed = v;
      if (fp) collapsed[f.path] = true;
    } catch (e) { showError('Could not save the mark', e); }
  }
  $effect(() => {
    const a = id;
    get<{ viewed: Record<string, string> }>(`/agents/${a}/viewed`).then((r) => { if (a === id) viewed = r.viewed; }, () => {});
  });
  let collapsed = $state<Record<string, boolean>>({});
  let previewing = $state<Record<string, boolean>>({});
  const previewKind = (p: string) => {
    const e = p.split('.').pop()?.toLowerCase() ?? '';
    return e === 'md' || e === 'markdown' ? 'markdown' : e === 'html' || e === 'htm' ? 'html'
      : ['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg'].includes(e) ? 'image' : null;
  };

  // Tabs: Uncommitted · All changes (branch against its base) · one commit.
  // Without a `view` in the URL the open gate decides, as before.
  type View = 'uncommitted' | 'branch' | 'commit';
  const gate = $derived(agent.gate);
  const defaultView = $derived<View>(
    gate?.kind === 'commit' ? 'commit' : gate?.kind === 'docs' && gate.role === 'ship' ? 'branch' : 'uncommitted',
  );
  const explicit = $derived(route.query.get('view') as View | null);
  const view = $derived<View>(explicit ?? defaultView);
  const commitId = $derived(
    route.query.get('commit') ?? (gate?.kind === 'commit' ? gate.id : agent.commits?.[agent.commits.length - 1]?.id ?? ''),
  );
  const pick = (v: View, c?: string) => go(`/agent/${agent.name}`, { s: 'changes', view: v, ...(c ? { commit: c } : {}) });
  const query = $derived(
    explicit ? `?view=${explicit}${explicit === 'commit' ? `&commit=${encodeURIComponent(commitId)}` : ''}` : '',
  );

  async function load() {
    loading = true;
    try { d = await get<DiffAnswer>(`/agents/${id}/diff${query}`); error = null; }
    catch (e) { error = (e as Error).message; }
    finally { loading = false; }
  }
  // Reload when what is reviewed changes (gate, commits, activity).
  const key = $derived(`${id}|${query}|${JSON.stringify(agent.gate ?? null)}|${agent.lastActivity}`);
  $effect(() => { void key; load(); });

  const files = $derived(d ? parseUnified(d.diff) : []);
  // Only the commit awaiting approval (no sha yet): its files are what is in the worktree.
  const canPreview = $derived(!!d?.commit && !d.commit.sha);
  const what = $derived(
    !d ? '' : d.commit ? (d.commit.sha ? `Commit ${d.commit.id} (${d.commit.sha.slice(0, 8)})` : `Commit ${d.commit.id}: the proposed files against HEAD`)
      : d.branch ? (d.view === 'branch' ? `Everything changed since the base (${d.branch})` : `The branch as it will be pushed (${d.branch})`)
      : 'Every uncommitted change',
  );
</script>

<div class="body">
  <div class="colmain">
    <div class="row">
      <Seg label="What to show" value={view} onchange={(v) => (v === 'commit' ? pick('commit', commitId) : pick(v))}
        options={[
          { value: 'uncommitted', label: 'Uncommitted' },
          { value: 'branch', label: 'All changes', disabled: !agent.base, title: agent.base ? '' : 'No base branch' },
          ...(agent.commits?.length ? [{ value: 'commit' as const, label: 'One commit' }] : []),
        ]} />
      {#if agent.commits?.length && view === 'commit'}
        <select aria-label="Commit" class="csel" value={commitId}
          onchange={(e) => e.currentTarget.value && pick('commit', e.currentTarget.value)}>
          {#each agent.commits as c (c.id)}<option value={c.id}>{c.id} · {c.subject} ({c.status})</option>{/each}
        </select>
      {/if}
    </div>
    {#if error}
      <div class="card cb"><p class="err">{error}</p></div>
    {:else if d}
      <div class="row">
        <h2 class="wrap-any">{what}</h2>
        <span class="grow"></span>
        <span class="mono muted">HEAD {d.head.slice(0, 8)}</span>
        {#if count}
          <button class="btn gate sm" onclick={sendNotes} disabled={sending || !agent.gate}
            title={agent.gate ? '' : 'The agent is not waiting at a gate'}>Send {count} comment{count === 1 ? '' : 's'}</button>
        {/if}
        <button class="btn sm" onclick={load} disabled={loading}>Refresh</button>
      </div>
      {#if !files.length}<div class="card cb"><p class="empty">No changes.</p></div>{/if}
      {#each files as f (f.path)}
        <section class="card">
          <div class="ch">
            <button class="btn ghost sm icon" aria-label={collapsed[f.path] ? 'Expand' : 'Collapse'} aria-expanded={!collapsed[f.path]} onclick={() => (collapsed[f.path] = !collapsed[f.path])}>
              {collapsed[f.path] ? '▸' : '▾'}
            </button>
            <span class="mono fpath">{f.path}</span>
            {#if f.status !== 'modified'}<span class="pill">{f.status}</span>{/if}
            <span class="grow"></span>
            {#if canPreview && previewKind(f.path) && f.status !== 'deleted'}
              <button class="btn sm" aria-pressed={!!previewing[f.path]} onclick={() => (previewing[f.path] = !previewing[f.path])}>
                {previewing[f.path] ? 'Diff' : 'Preview'}
              </button>
            {/if}
            <label class="f chk"><input type="checkbox" checked={isViewed(f)} onchange={() => toggleViewed(f)} /> Viewed</label>
            <span class="num adds">+{f.adds}</span><span class="num dels">−{f.dels}</span>
          </div>
          {#if !collapsed[f.path]}
            {@const k = previewKind(f.path)}
            {#if previewing[f.path] && k}<FilePreview {id} path={f.path} kind={k} notes={mdNotes[f.path]} onnote={setMd(f.path)} />{:else}<DiffLines lines={f.lines} notes={notes[f.path]} onnote={setNote(f.path)} />{/if}
          {/if}
        </section>
      {/each}
    {:else}
      <div class="card cb"><p class="empty">Loading…</p></div>
    {/if}
  </div>
  {#if d && (d.files.length || d.others.length || d.commit)}
    <aside class="colside">
      {#if d.commit}
        <section class="card cb">
          <span class="lbl">Commit message</span>
          <pre class="mono pre msg">{d.commit.message ?? d.commit.subject}</pre>
        </section>
      {/if}
      {#if d.files.length}
        <section class="card cb">
          <span class="lbl">{d.commit ? 'In this commit' : 'Files'} · {d.files.length}</span>
          <ul class="mono">{#each d.files as f (f)}<li class="wrap-any">{f}</li>{/each}</ul>
        </section>
      {/if}
      {#if d.others.length && (d.commit || d.branch)}
        <section class="card cb">
          <span class="lbl">{d.branch ? 'Uncommitted, not pushed' : 'Left out of this commit'} · {d.others.length}</span>
          <ul class="mono muted">{#each d.others as f (f)}<li class="wrap-any">{f}</li>{/each}</ul>
        </section>
      {/if}
    </aside>
  {/if}
</div>

<style>
  .csel { width: auto; max-width: 100%; }
  .fpath { overflow-wrap: anywhere; min-width: 0; flex: 1 1 200px; }
  .adds { color: var(--live); font-weight: 600; }
  .dels { color: var(--halt); font-weight: 600; }
  .msg { margin: 6px 0 0; font-size: 12.5px; line-height: 1.45; }
</style>
