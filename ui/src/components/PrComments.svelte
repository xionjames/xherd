<script lang="ts">
  // The comments of the current PR round and what to do about each (R-306):
  // the agent proposes in its plan, the user decides here and sends the
  // decisions (approves the PR gate). Comment text is untrusted: plain text.
  import type { Agent, Decision, PrComment, Verdict } from '../lib/types';
  import { get, post, put } from '../lib/api';
  import { upsertAgent } from '../lib/store.svelte';
  import { showError } from '../lib/errors.svelte';
  import { prGate } from '../lib/flow';
  import Seg from './Seg.svelte';

  let { agent }: { agent: Agent } = $props();
  // Effects key on the id: the agent object is replaced on every update.
  const id = $derived(agent.id);
  // Reload when the plan is rewritten or the round moves on.
  const key = $derived(
    (agent.gate?.kind === 'docs' ? agent.gate.files.map((f) => f.sha256).join() : '') + agent.status,
  );

  interface Round {
    comments: PrComment[];
    proposals: Verdict[];
    overrides: Record<string, Verdict>;
    posted: string[];
    error?: string | null;
  }
  let round = $state<Round | null>(null);
  let overrides = $state<Record<string, Verdict>>({});
  let error = $state<string | null>(null);

  const GLYPH: Record<Decision, string> = { fix: '✎', reply: '↩', skip: '○' };
  const TONE: Record<Decision, string> = { fix: 'acc', reply: 'live', skip: '' };
  const DECISIONS: Decision[] = ['fix', 'reply', 'skip'];

  $effect(() => {
    void key;
    get<Round>(`/agents/${id}/pr-review`).then(
      (r) => { round = r; overrides = { ...r.overrides }; error = null; },
      (e) => { round = null; error = e.message; },
    );
  });

  const proposal = (c: PrComment) => round?.proposals.find((p) => p.id === c.id);
  const effective = (c: PrComment): Verdict | undefined => overrides[c.id] ?? proposal(c);
  const posted = (c: PrComment) => round?.posted.includes(c.id) ?? false;
  const location = (c: PrComment) => (c.path ? `${c.path}${c.line ? `:${c.line}` : ''}` : c.kind);
  const safe = (u?: string) => (u && /^https:\/\//.test(u) ? u : undefined);

  async function save() {
    try {
      await put(`/agents/${id}/pr-decisions`, { decisions: overrides });
      error = null;
    } catch (e) {
      error = (e as Error).message;
    }
  }
  /** Start from what is shown (the proposal) so a change keeps its note and reply. */
  function edit(c: PrComment, change: Partial<Verdict>) {
    const base = effective(c) ?? { id: c.id, decision: 'skip' as Decision };
    overrides = { ...overrides, [c.id]: { ...base, ...change } };
    void save();
  }
  const open = $derived((round?.comments ?? []).length);

  // Sending = approving the PR gate: replies are posted, fixes go to the agent.
  const deciding = $derived(prGate(agent.gate));
  const missing = $derived(
    (round?.comments ?? []).filter((c) => {
      const v = effective(c);
      return !v || (v.decision === 'reply' && !v.reply?.trim());
    }).length,
  );
  const count = (d: Decision) => (round?.comments ?? []).filter((c) => effective(c)?.decision === d).length;
  let sending = $state(false);
  async function send() {
    sending = true;
    try {
      const r = await post<{ agent: Agent }>(`/agents/${id}/approve`, { expect: {} });
      upsertAgent(r.agent);
    } catch (e) {
      showError('Could not send the decisions', e);
    } finally {
      sending = false;
    }
  }
</script>

{#if error}
  <div class="card cb"><p class="err">{error}</p></div>
{/if}
{#if round && open}
  <section class="card" aria-label="Pull request comments">
    <div class="ch">
      <h2>Comments to decide</h2>
      <span class="muted">{open} in this round{#if deciding}: check what to do with each, then send the decisions{/if}</span>
    </div>
    {#if round.error}<div class="cb"><p class="err">pr-review.md: {round.error}</p></div>{/if}
    <ul class="list">
      {#each round.comments as c (c.id)}
        {@const v = effective(c)}
        <li class="dec {v?.decision ?? 'none'}">
          <div class="row">
            <b>{c.author}</b>
            <span class="mono muted wrap-any">{location(c)}</span>
            {#if safe(c.url)}<a href={safe(c.url)} target="_blank" rel="noopener noreferrer">open</a>{/if}
            <span class="grow"></span>
            {#if posted(c)}<span class="pill live">✓ reply posted</span>{/if}
          </div>
          <p class="pre wrap-any">{c.body}</p>
          <div class="row">
            <Seg label="Decision for this comment" value={v?.decision ?? ('' as Decision)} onchange={(d) => edit(c, { decision: d })}
              options={DECISIONS.map((d) => ({ value: d, label: `${GLYPH[d]} ${d}`, tone: TONE[d] }))} />
            {#if v && !overrides[c.id]}<span class="muted">proposed by the agent</span>{/if}
            {#if !v}<span class="pill gate">no decision yet</span>{/if}
          </div>
          {#if v}
            <input placeholder="Note (why)" value={v.note ?? ''}
              onchange={(e) => edit(c, { note: e.currentTarget.value })} />
            {#if v.decision !== 'skip'}
              <textarea rows="2"
                placeholder={v.decision === 'reply' ? 'Reply to post (required)' : 'Reply draft (not posted)'}
                value={v.reply ?? ''} onchange={(e) => edit(c, { reply: e.currentTarget.value })}></textarea>
            {/if}
            {#if v.decision === 'reply'}<span class="hint">Posted on the pull request when you approve, marked as sent by xherd.</span>{/if}
          {/if}
        </li>
      {/each}
    </ul>
    {#if deciding}
      <div class="cf">
        <span class="muted">{count('fix')} to fix · {count('reply')} to reply · {count('skip')} to skip{#if missing} · <span class="gtext">{missing} still need{missing === 1 ? 's' : ''} a decision or reply text</span>{/if}</span>
        <span class="grow"></span>
        <button class="btn gate" disabled={sending || missing > 0 || agent.status === 'blocked'} onclick={send}>Send decisions to the agent</button>
      </div>
    {/if}
  </section>
{/if}


<style>
  /* The decision shows on the comment's edge too: fix cobalt, reply green. */
  .dec { border-left: 3px solid transparent; }
  .dec.fix { border-left-color: var(--accent); }
  .dec.reply { border-left-color: var(--live); }
  .dec.none { border-left-color: var(--gate); }
  .gtext { color: var(--gate); }
</style>
