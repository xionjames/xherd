<script lang="ts">
  // TODOs (spec §16.2): reminders, once or repeating. One that has come due
  // needs you: complete it, snooze it, edit or delete it. Condition checks
  // (a read-only runtime session) are not built.
  import { get, post, put, del } from '../lib/api';
  import { agoText } from '../lib/format';
  import { showError } from '../lib/errors.svelte';

  type Todo = { id: string; title: string; note?: string; due: number; repeatMinutes?: number; status: 'open' | 'due' | 'done'; createdAt: number; completedAt?: number };
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
  let form = $state({ title: '', note: '', due: inAnHour(), repeat: '' });
  let editing = $state<string | null>(null);
  let busy = $state(false);

  const body = () => ({
    title: form.title.trim(),
    note: form.note.trim() || undefined,
    due: new Date(form.due).getTime(),
    repeatMinutes: form.repeat ? Number(form.repeat) : undefined,
  });
  async function run(f: () => Promise<unknown>, what: string) {
    busy = true;
    try { await f(); await load(); } catch (e) { showError(what, e); } finally { busy = false; }
  }
  const save = () => run(async () => {
    if (editing) await put(`/todos/${editing}`, body()); else await post('/todos', body());
    editing = null;
    form = { title: '', note: '', due: inAnHour(), repeat: '' };
  }, 'Could not save the TODO');
  function edit(t: Todo) {
    editing = t.id;
    form = { title: t.title, note: t.note ?? '', due: local(t.due), repeat: t.repeatMinutes ? String(t.repeatMinutes) : '' };
  }
  const snooze = (t: Todo, ms: number) => run(() => post(`/todos/${t.id}/reschedule`, { due: Date.now() + ms }), 'Could not reschedule');
  const when = (ms: number) => new Date(ms).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
  const repeatLabel = (m?: number) => (m ? (REPEATS.find(([v]) => v === String(m))?.[1] ?? `every ${m} min`).toLowerCase() : '');

  const due = $derived((todos ?? []).filter((t) => t.status === 'due'));
  const open = $derived((todos ?? []).filter((t) => t.status === 'open').sort((a, b) => a.due - b.due));
  const done = $derived((todos ?? []).filter((t) => t.status === 'done').sort((a, b) => (b.completedAt ?? 0) - (a.completedAt ?? 0)).slice(0, 20));
</script>

<div class="stack">
  <div class="phead notabs"><div class="row"><h1>TODOs</h1></div></div>
  <p class="muted">Reminders for you, not for agents. When one comes due it needs you and you get a notification.</p>

  <form class="card f" style="padding:12px 16px" onsubmit={(e) => { e.preventDefault(); save(); }}>
    <h2>{editing ? 'Edit the TODO' : 'New TODO'}</h2>
    <label class="f">Title <input bind:value={form.title} maxlength="200" required placeholder="Renew the TLS certificate" /></label>
    <label class="f">Note <textarea bind:value={form.note} maxlength="2000" rows="2"></textarea></label>
    <div class="row" style="gap:12px;flex-wrap:wrap">
      <label class="f">Due <input type="datetime-local" bind:value={form.due} required /></label>
      <label class="f">Repeat <select bind:value={form.repeat}>{#each REPEATS as [v, l] (v)}<option value={v}>{l}</option>{/each}</select></label>
    </div>
    <div class="row" style="gap:8px">
      <button class="btn gate" type="submit" disabled={busy || !form.title.trim()}>{editing ? 'Save' : 'Add'}</button>
      {#if editing}<button class="btn ghost" type="button" onclick={() => { editing = null; form = { title: '', note: '', due: inAnHour(), repeat: '' }; }}>Cancel</button>{/if}
    </div>
  </form>

  {#if todos === null}
    <p class="muted">Loading…</p>
  {:else}
    {#snippet item(t: Todo)}
      <li class="it">
        <div class="row" style="gap:10px;flex-wrap:wrap">
          <b>{t.title}</b>
          <span class="hint">{t.status === 'done' ? `done ${agoText(t.completedAt ?? t.due)}` : `${t.status === 'due' ? 'was due' : 'due'} ${when(t.due)}`}{t.repeatMinutes ? ` · ${repeatLabel(t.repeatMinutes)}` : ''}</span>
          <span class="grow"></span>
          {#if t.status !== 'done'}
            <button class="btn gate sm" disabled={busy} onclick={() => run(() => post(`/todos/${t.id}/complete`), 'Could not complete')}>Complete</button>
            <button class="btn sm" disabled={busy} onclick={() => snooze(t, 3600000)}>Snooze 1 h</button>
            <button class="btn sm" disabled={busy} onclick={() => snooze(t, 86400000)}>Tomorrow</button>
            <button class="btn ghost sm" onclick={() => edit(t)}>Edit</button>
          {/if}
          <button class="btn ghost sm" disabled={busy} onclick={() => run(() => del(`/todos/${t.id}`), 'Could not delete')}>Delete</button>
        </div>
        {#if t.note}<div class="hint" style="white-space:pre-wrap">{t.note}</div>{/if}
      </li>
    {/snippet}
    <section class="stack">
      <h2>Needs you <span class="hint num">{due.length}</span></h2>
      {#if !due.length}<p class="muted">Nothing is due.</p>{:else}<ul class="card list">{#each due as t (t.id)}{@render item(t)}{/each}</ul>{/if}
    </section>
    <section class="stack">
      <h2>Upcoming <span class="hint num">{open.length}</span></h2>
      {#if !open.length}<p class="muted">No upcoming TODOs.</p>{:else}<ul class="card list">{#each open as t (t.id)}{@render item(t)}{/each}</ul>{/if}
    </section>
    {#if done.length}
      <section class="stack"><h2>Done</h2><ul class="card list">{#each done as t (t.id)}{@render item(t)}{/each}</ul></section>
    {/if}
  {/if}
</div>

<style>
  .list { list-style: none; margin: 0; padding: 0; }
  .it { padding: 10px 16px; border-bottom: 1px solid var(--fence-soft); }
</style>
