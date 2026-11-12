<script lang="ts">
  // The agent's ticket and its comments, read through a tracker plugin
  // (github-issues, gitlab-issues, or an external one). Read-only: xherd posts
  // nothing to the tracker. The text is written by other people, so it is
  // rendered through the same sanitiser as agent documents.
  import Panel from '../components/Panel.svelte';
  import type { Agent } from '../lib/types';
  import { get, post } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { render } from '../lib/markdown';
  import { route } from '../lib/router.svelte';
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
  // Arriving from the "pull request is open" prompt: the draft is ready.
  let autoDrafted = $state(false);
  $effect(() => {
    if (!autoDrafted && answer?.ticket && route.query.get('draft') === '1') { autoDrafted = true; makeDraft(); }
  });

  const https = (u?: string) => (u && /^https:\/\//.test(u) ? u : undefined);
  const when = (iso?: string) => { const t = iso ? Date.parse(iso) : NaN; return Number.isNaN(t) ? '' : agoText(t); };
  const t = $derived(answer?.ticket);

  // A short summary of what the agent did, drafted by xherd. It is posted only
  // when the user presses Post, with the text as edited here (R-308).
  let draft = $state<string | null>(null);
  let posting = $state(false);
  let posted = $state(false);
  async function makeDraft() {
    try {
      draft = (await get<{ text: string }>(`/agents/${id}/ticket-summary`)).text;
      posted = false;
    } catch (e) { showError('Could not draft a summary', e); }
  }
  async function postDraft() {
    if (!draft?.trim()) return;
    posting = true;
    try {
      await post(`/agents/${id}/ticket-comment`, { body: draft });
      posted = true;
      draft = null;
      await load();
    } catch (e) { showError('Could not post the comment', e); }
    finally { posting = false; }
  }
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
    <Panel title="Summary for the ticket" hint="xherd drafts it from the finished tasks, commits and pull request. You read and edit it; nothing is posted until you press Post.">
      {#if draft === null}
        {#if posted}<p class="hint">✓ Posted as a comment on the ticket.</p>{/if}
        <div class="row"><button class="btn" onclick={makeDraft}>{posted ? 'Draft another summary' : 'Draft a summary'}</button></div>
      {:else}
        <label class="f">Comment to post on {answer?.tracker} #{answer?.issue}
          <textarea rows="9" bind:value={draft} spellcheck="true"></textarea></label>
        <div class="row">
          <button class="btn primary" disabled={posting || !draft.trim()} onclick={postDraft}>{posting ? 'Posting…' : 'Post to the ticket'}</button>
          <button class="btn ghost" disabled={posting} onclick={() => (draft = null)}>Cancel</button>
        </div>
      {/if}
    </Panel>
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
