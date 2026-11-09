<script lang="ts">
  // The comments of the current PR round and what to do about each (R-306):
  // the agent proposes in pr-review.md, the user decides here, then approves
  // the pr-review gate. Comment text is untrusted: it is shown as plain text.
  import type { Agent, Decision, PrComment, Verdict } from '../lib/types';
  import { get, put } from '../lib/api';

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
</script>

{#if error}
  <div class="card cb" style="color:var(--halt)">{error}</div>
{/if}
{#if round && open}
  <section class="card" aria-label="Pull request comments">
    <div class="ch">
      <h2>Comments to decide</h2>
      <span class="muted">{open} in this round: choose what to do, then approve the plan</span>
    </div>
    {#if round.error}<div class="cb" style="color:var(--halt)">pr-review.md: {round.error}</div>{/if}
    <ul class="list">
      {#each round.comments as c (c.id)}
        {@const v = effective(c)}
        <li class="item">
          <div class="row">
            <b>{c.author}</b>
            <span class="mono muted">{location(c)}</span>
            {#if safe(c.url)}<a href={safe(c.url)} target="_blank" rel="noopener noreferrer">open</a>{/if}
            <span class="grow"></span>
            {#if posted(c)}<span class="pill live">✓ reply posted</span>{/if}
          </div>
          <p class="body">{c.body}</p>
          <div class="row" role="group" aria-label="Decision for this comment">
            {#each DECISIONS as d (d)}
              <button class="chip" class:on={v?.decision === d} aria-pressed={v?.decision === d}
                onclick={() => edit(c, { decision: d })}>{GLYPH[d]} {d}</button>
            {/each}
            {#if v && !overrides[c.id]}<span class="muted">proposed by the agent</span>{/if}
            {#if !v}<span class="pill gate">no decision yet</span>{/if}
          </div>
          {#if v}
            <input class="note" placeholder="Note (why)" value={v.note ?? ''}
              onchange={(e) => edit(c, { note: e.currentTarget.value })} />
            {#if v.decision !== 'skip'}
              <textarea class="reply" rows="2"
                placeholder={v.decision === 'reply' ? 'Reply to post (required)' : 'Reply draft (not posted)'}
                value={v.reply ?? ''} onchange={(e) => edit(c, { reply: e.currentTarget.value })}></textarea>
            {/if}
            {#if v.decision === 'reply'}<span class="hint">Posted on the pull request when you approve, marked as sent by xherd.</span>{/if}
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .list { list-style: none; margin: 0; padding: 0; }
  .item { padding: 10px 14px; border-top: 1px solid var(--fence-soft); display: flex; flex-direction: column; gap: 6px; }
  .body { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .note, .reply { width: 100%; font: 13px var(--sans); color: var(--ink); background: var(--well); border: 1px solid var(--fence); border-radius: var(--r-ctl); padding: 6px 10px; }
</style>
