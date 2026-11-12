<script lang="ts">
  // The agent's ticket and its comments, read through a tracker plugin
  // (github-issues, gitlab-issues, or an external one). Read-only: xherd posts
  // nothing to the tracker. The text is written by other people, so it is
  // rendered through the same sanitiser as agent documents.
  import Panel from '../components/Panel.svelte';
  import type { Agent } from '../lib/types';
  import { get } from '../lib/api';
  import { render } from '../lib/markdown';
  import { agoText } from '../lib/format';

  let { agent }: { agent: Agent } = $props();
  // Effects key on the id: the agent object is replaced on every update.
  const id = $derived(agent.id);

  interface Comment { id: string; author: string; body: string; url?: string; createdAt?: string }
  interface View { title: string; body: string; state: string; url: string; author: string; comments: Comment[] }
  interface Answer { tracker: string | null; key?: string; issue?: string; ticket?: View }

  let answer = $state<Answer | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);

  async function load() {
    loading = true;
    try {
      answer = await get<Answer>(`/agents/${id}/ticket`);
      error = null;
    } catch (e) {
      answer = null;
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => { void id; load(); });

  const https = (u?: string) => (u && /^https:\/\//.test(u) ? u : undefined);
  const when = (iso?: string) => { const t = iso ? Date.parse(iso) : NaN; return Number.isNaN(t) ? '' : agoText(t); };
  const t = $derived(answer?.ticket);
</script>

<div class="stack">
  <Panel title="Ticket" hint={`Read through the tracker plugin; nothing is posted back.`}>
    {#snippet head()}
      <span class="grow"></span>
      <button class="btn sm" disabled={loading} onclick={load}>{loading ? 'Loading…' : 'Refresh'}</button>
    {/snippet}
    {#if error}
      <p class="hint err wrap-any">{error}</p>
    {:else if !answer}
      <p class="empty">Loading…</p>
    {:else if answer.tracker === null}
      <p class="empty">No issue tracker reads the ticket key <span class="mono">{answer.key}</span>. Switch on GitHub Issues or GitLab Issues in <a href="#/settings?tab=plugins">Settings → Plugins</a> (their keys are the issue number, with an optional GH- or GL- prefix).</p>
    {:else if t}
      <div class="row">
        <h3 class="grow wrap-any">{t.title}</h3>
        <span class="pill {t.state === 'open' ? 'live' : 'done'}">{t.state || 'unknown'}</span>
      </div>
      <p class="hint">
        {answer.tracker} · #{answer.issue}{#if t.author} · by {t.author}{/if}
        {#if https(t.url)} · <a href={https(t.url)} target="_blank" rel="noopener noreferrer">open</a>{/if}
      </p>
      {#if t.body.trim()}<div class="md">{@html render(t.body).html}</div>{:else}<p class="empty">No description.</p>{/if}
    {/if}
  </Panel>

  {#if t}
    <Panel title={`Comments (${t.comments.length})`}>
      {#if !t.comments.length}
        <p class="empty">No comments.</p>
      {/if}
      {#each t.comments as c (c.id)}
        <div class="grp">
          <p class="row"><b>{c.author}</b>
            {#if when(c.createdAt)}<span class="hint">{when(c.createdAt)}</span>{/if}
            {#if https(c.url)}<a class="hint" href={https(c.url)} target="_blank" rel="noopener noreferrer">link</a>{/if}</p>
          <div class="md">{@html render(c.body).html}</div>
        </div>
      {/each}
    </Panel>
  {/if}
</div>
