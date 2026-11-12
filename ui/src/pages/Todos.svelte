<script lang="ts">
  // TODOs (spec §16.2): reminders, once or repeating. One that has come due
  // needs you: complete it, edit or delete it. A condition TODO is
  // checked at each due time by a short read-only runtime run: met needs you,
  // not met is checked again later.
  import { get, post, put, del } from '../lib/api';
  import { agoText } from '../lib/format';
  import { showError } from '../lib/errors.svelte';
  import PageHead from '../components/PageHead.svelte';
  import Panel from '../components/Panel.svelte';

  type LastCheck = { at: number; outcome: 'met' | 'not-met' | 'unclear' | 'failed'; summary?: string };
  type Todo = { id: string; title: string; note?: string; due: number; repeatMinutes?: number; condition?: string; model?: string; status: 'open' | 'checking' | 'due' | 'done' | 'error'; createdAt: number; completedAt?: number; lastCheck?: LastCheck };
  let todos = $state<Todo[] | null>(null);
  const load = () => get<{ todos: Todo[] }>('/todos').then((r) => (todos = r.todos)).catch((e) => showError('Could not load the TODOs', e));
  $effect(() => {
    load();
    const t = setInterval(load, 30000);
    return () => clearInterval(t);
  });

  // Repeat = a number and a unit (minutes per unit); the daemon stores minutes.
  const UNITS: [number, string][] = [[1, 'minutes'], [60, 'hours'], [1440, 'days'], [10080, 'weeks']];
  const split = (m?: number): { every: string; unit: number } => {
    if (!m) return { every: '', unit: 60 };
    const [u] = [...UNITS].reverse().find(([u]) => m % u === 0) ?? UNITS[0];
    return { every: String(m / u), unit: u };
  };
  // `datetime-local` wants local time without a zone.
  const local = (ms: number) => { const d = new Date(ms - new Date(ms).getTimezoneOffset() * 60000); return d.toISOString().slice(0, 16); };
  const inAnHour = () => local(Date.now() + 3600000);
  const blank = () => ({ title: '', due: inAnHour(), every: '', unit: 60, condition: '', model: '' });
  let form = $state(blank());
  // One form: the condition is optional. With one, the TODO is checked at each time and only needs you when it is met, so it needs a check interval.
  const hasCond = $derived(form.condition.trim() !== '');
  $effect(() => {
    if (hasCond && !form.every) { form.every = '1'; form.unit = 60; }
  });
  const repeatMinutes = $derived(form.every ? Math.round(Number(form.every) * form.unit) : 0);
  let showForm = $state(false);
  let editing = $state<string | null>(null);
  let busy = $state(false);

  const body = () => ({
    title: form.title.trim(),
    due: new Date(form.due).getTime(),
    repeatMinutes: repeatMinutes || undefined,
    condition: hasCond ? form.condition.trim() : undefined,
    model: hasCond && form.model.trim() ? form.model.trim() : undefined,
  });
  async function run(f: () => Promise<unknown>, what: string) {
    busy = true;
    try { await f(); await load(); } catch (e) { showError(what, e); } finally { busy = false; }
  }
  const save = () => run(async () => {
    if (editing) await put(`/todos/${editing}`, body()); else await post('/todos', body());
    editing = null;
    form = blank();
    showForm = false;
  }, 'Could not save the TODO');
  const close = () => { editing = null; form = blank(); showForm = false; };
  function edit(t: Todo) {
    editing = t.id;
    form = { title: t.title, due: local(t.due), ...split(t.repeatMinutes), condition: t.condition ?? '', model: t.model ?? '' };
    showForm = true;
  }
  const when = (ms: number) => new Date(ms).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
  const repeatLabel = (m?: number) => {
    if (!m) return '';
    const { every, unit } = split(m);
    const name = UNITS.find(([u]) => u === unit)![1];
    return every === '1' ? `every ${name.replace(/s$/, '')}` : `every ${every} ${name}`;
  };
  const OUTCOME: Record<LastCheck['outcome'], string> = { met: 'met', 'not-met': 'not met', unclear: 'no clear answer', failed: 'failed' };

  const due = $derived((todos ?? []).filter((t) => t.status === 'due' || t.status === 'error'));
  const open = $derived((todos ?? []).filter((t) => t.status === 'open' || t.status === 'checking').sort((a, b) => a.due - b.due));
  const done = $derived((todos ?? []).filter((t) => t.status === 'done').sort((a, b) => (b.completedAt ?? 0) - (a.completedAt ?? 0)).slice(0, 20));
</script>

<PageHead width="mid" title="TODOs" sub="Reminders for you, not for agents. When one comes due it needs you and you get a notification. A reminder with a condition only comes due when the condition is met.">
  {#snippet actions()}
    {#if !showForm}<button class="btn primary" onclick={() => (showForm = true)}>New TODO</button>{/if}
  {/snippet}
</PageHead>

<div class="stack mid">
  {#if showForm}
  <form onsubmit={(e) => { e.preventDefault(); save(); }}>
    <Panel title={editing ? 'Edit the TODO' : 'New TODO'}>
      <label class="f">Title <input bind:value={form.title} maxlength="200" required placeholder="Renew the TLS certificate" /></label>
      <label class="f">Condition (optional) <textarea bind:value={form.condition} maxlength="1000" rows="2" placeholder="The pull request for PROJ-123 is merged"></textarea>
        <span class="hint">Leave empty for a plain reminder. With a condition, at each time a short read-only run of Claude Code looks into it (it can read files and use <span class="mono">gh</span> and <span class="mono">git</span> to read, nothing that changes anything). You are notified only when it is met; otherwise it is checked again at the next interval.</span></label>
      {#if hasCond}
        <label class="f">Model <input class="mono" bind:value={form.model} maxlength="100" placeholder="the runtime's default" /></label>
      {/if}
      <div class="g2">
        <label class="f">Due <input type="datetime-local" bind:value={form.due} required /></label>
        <div class="f">{hasCond ? 'Check again every' : 'Repeat every (empty = once)'}
          <div class="row">
            <input type="number" min="1" step="any" bind:value={form.every} required={hasCond} aria-label="Repeat interval" />
            <select bind:value={form.unit} aria-label="Repeat unit">{#each UNITS as [u, n] (u)}<option value={u}>{n}</option>{/each}</select>
          </div>
          {#if form.every && repeatMinutes < 5}<span class="hint">At least 5 minutes.</span>{/if}
        </div>
      </div>
      {#snippet foot()}
        <button class="btn ghost" type="button" onclick={close}>Cancel</button>
        <button class="btn primary" type="submit" disabled={busy || !form.title.trim() || (!!form.every && repeatMinutes < 5)}>{editing ? 'Save TODO' : 'Add TODO'}</button>
      {/snippet}
    </Panel>
  </form>
  {/if}

  {#if todos === null}
    <p class="empty">Loading…</p>
  {:else}
    {#snippet item(t: Todo)}
      <li>
        <div class="row">
          <b class="wrap-any">{t.title}</b>
          <span class="hint">{t.status === 'done' ? `done ${agoText(t.completedAt ?? t.due)}` : t.status === 'checking' ? 'checking now…' : `${t.status === 'due' || t.status === 'error' ? 'was due' : t.condition ? 'next check' : 'due'} ${when(t.due)}`}{t.repeatMinutes ? ` · ${repeatLabel(t.repeatMinutes)}` : ''}</span>
        </div>
        {#if t.condition}<div class="hint">Condition: {t.condition}{t.model ? ` · model ${t.model}` : ''}</div>{/if}
        {#if t.lastCheck}<div class="hint">Last check {agoText(t.lastCheck.at)}: {OUTCOME[t.lastCheck.outcome]}{t.lastCheck.summary ? ` — ${t.lastCheck.summary}` : ''}</div>{/if}
        {#if t.note}<div class="hint pre">{t.note}</div>{/if}
        <div class="row">
          {#if t.status !== 'done'}
            <button class="btn sm" class:gate={t.status === 'due' || t.status === 'error'} disabled={busy} onclick={() => run(() => post(`/todos/${t.id}/complete`), 'Could not complete')}>Complete</button>
            {#if t.condition}<button class="btn sm" disabled={busy || t.status === 'checking'} onclick={() => run(() => post(`/todos/${t.id}/check`), 'Could not start the check')}>Check now</button>{/if}
            {#if (t.status === 'due' || t.status === 'error') && t.repeatMinutes}<button class="btn sm" disabled={busy} title="Clear the alert and keep the TODO: it is checked again after one interval" onclick={() => run(() => post(`/todos/${t.id}/reschedule`, { due: Date.now() + t.repeatMinutes! * 60000 }), 'Could not dismiss')}>Ignore</button>{/if}
            <button class="btn ghost sm" onclick={() => edit(t)}>Edit</button>
          {/if}
          <button class="btn ghost danger sm" disabled={busy} onclick={() => run(() => del(`/todos/${t.id}`), 'Could not delete')}>Delete</button>
        </div>
      </li>
    {/snippet}
    <Panel title="Needs you" tone={due.length ? 'need' : undefined} flush>
      {#snippet head()}<span class="pill" class:gate={due.length > 0}>{due.length}</span>{/snippet}
      {#if !due.length}<div class="cb"><p class="empty">Nothing is due.</p></div>{:else}<ul class="list">{#each due as t (t.id)}{@render item(t)}{/each}</ul>{/if}
    </Panel>
    <Panel title="Upcoming" flush>
      {#snippet head()}<span class="pill">{open.length}</span>{/snippet}
      {#if !open.length}<div class="cb"><p class="empty">No upcoming TODOs.</p></div>{:else}<ul class="list">{#each open as t (t.id)}{@render item(t)}{/each}</ul>{/if}
    </Panel>
    {#if done.length}
      <Panel title="Done" flush><ul class="list">{#each done as t (t.id)}{@render item(t)}{/each}</ul></Panel>
    {/if}
  {/if}
</div>
