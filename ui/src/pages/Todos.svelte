<script lang="ts">
  // TODOs (spec §16.2): reminders, once or repeating. One that has come due
  // needs you: complete it, snooze it, edit or delete it. A condition TODO is
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

  const REPEATS: [string, string][] = [['', 'Once'], ['60', 'Every hour'], ['1440', 'Every day'], ['10080', 'Every week']];
  // `datetime-local` wants local time without a zone.
  const local = (ms: number) => { const d = new Date(ms - new Date(ms).getTimezoneOffset() * 60000); return d.toISOString().slice(0, 16); };
  const inAnHour = () => local(Date.now() + 3600000);
  const CHECKS: [string, string][] = [['15', 'Every 15 minutes'], ['60', 'Every hour'], ['1440', 'Every day']];
  const blank = () => ({ title: '', note: '', due: inAnHour(), repeat: '', condition: '', model: '' });
  let form = $state(blank());
  // One form: the condition is optional. With one, the TODO is checked at each time and only needs you when it is met, so it needs a check interval.
  const hasCond = $derived(form.condition.trim() !== '');
  $effect(() => {
    if (hasCond && !CHECKS.some(([v]) => v === form.repeat)) form.repeat = '60';
    else if (!hasCond && form.repeat === '15') form.repeat = '';
  });
  let editing = $state<string | null>(null);
  let busy = $state(false);

  const body = () => ({
    title: form.title.trim(),
    note: form.note.trim() || undefined,
    due: new Date(form.due).getTime(),
    repeatMinutes: form.repeat ? Number(form.repeat) : undefined,
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
  }, 'Could not save the TODO');
  function edit(t: Todo) {
    editing = t.id;
    form = { title: t.title, note: t.note ?? '', due: local(t.due), repeat: t.repeatMinutes ? String(t.repeatMinutes) : '', condition: t.condition ?? '', model: t.model ?? '' };
  }
  const snooze = (t: Todo, ms: number) => run(() => post(`/todos/${t.id}/reschedule`, { due: Date.now() + ms }), 'Could not reschedule');
  const when = (ms: number) => new Date(ms).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
  const repeatLabel = (m?: number) => (m ? ([...REPEATS, ...CHECKS].find(([v]) => v === String(m))?.[1] ?? `every ${m} min`).toLowerCase() : '');
  const OUTCOME: Record<LastCheck['outcome'], string> = { met: 'met', 'not-met': 'not met', unclear: 'no clear answer', failed: 'failed' };

  const due = $derived((todos ?? []).filter((t) => t.status === 'due' || t.status === 'error'));
  const open = $derived((todos ?? []).filter((t) => t.status === 'open' || t.status === 'checking').sort((a, b) => a.due - b.due));
  const done = $derived((todos ?? []).filter((t) => t.status === 'done').sort((a, b) => (b.completedAt ?? 0) - (a.completedAt ?? 0)).slice(0, 20));
</script>

<PageHead title="TODOs" sub="Reminders for you, not for agents. When one comes due it needs you and you get a notification. A reminder with a condition only comes due when the condition is met." />

<div class="stack mid">
  <form onsubmit={(e) => { e.preventDefault(); save(); }}>
    <Panel title={editing ? 'Edit the TODO' : 'New TODO'}>
      <label class="f">Title <input bind:value={form.title} maxlength="200" required placeholder="Renew the TLS certificate" /></label>
      <label class="f">Condition (optional) <textarea bind:value={form.condition} maxlength="1000" rows="2" placeholder="The pull request for PROJ-123 is merged"></textarea>
        <span class="hint">Leave empty for a plain reminder. With a condition, at each time a short read-only run of Claude Code looks into it (it can read files and use <span class="mono">gh</span> and <span class="mono">git</span> to read, nothing that changes anything). You are notified only when it is met; otherwise it is checked again at the next interval.</span></label>
      {#if hasCond}
        <label class="f">Model <input class="mono" bind:value={form.model} maxlength="100" placeholder="the runtime's default" /></label>
      {/if}
      <label class="f">Note <textarea bind:value={form.note} maxlength="2000" rows="2"></textarea></label>
      <div class="g2">
        <label class="f">Due <input type="datetime-local" bind:value={form.due} required /></label>
        <label class="f">{hasCond ? 'Check again' : 'Repeat'} <select bind:value={form.repeat}>{#each hasCond ? CHECKS : REPEATS as [v, l] (v)}<option value={v}>{l}</option>{/each}</select></label>
      </div>
      {#snippet foot()}
        {#if editing}<button class="btn ghost" type="button" onclick={() => { editing = null; form = blank(); }}>Cancel</button>{/if}
        <button class="btn primary" type="submit" disabled={busy || !form.title.trim()}>{editing ? 'Save TODO' : 'Add TODO'}</button>
      {/snippet}
    </Panel>
  </form>

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
            <button class="btn sm" disabled={busy} onclick={() => snooze(t, 3600000)}>Snooze 1 h</button>
            <button class="btn sm" disabled={busy} onclick={() => snooze(t, 86400000)}>Tomorrow</button>
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
