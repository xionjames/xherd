<script lang="ts">
  import Panel from '../components/Panel.svelte';
  import type { Agent, PrCheck, PrReviewer } from '../lib/types';
  import { agoText } from '../lib/format';
  import PrComments from '../components/PrComments.svelte';
  let { agent }: { agent: Agent } = $props();
  const STATE_TONE = { open: 'acc', merged: 'done', closed: '' };
  const STATE_GLYPH = { open: '●', merged: '✓', closed: '○' };
  const CI_TONE = { pending: '', failing: 'halt', passing: 'live' };
  const CI_GLYPH = { pending: '…', failing: '✕', passing: '✓' };
  // Each check result: tone, glyph and label; failing ones come first.
  const BUCKET: Record<string, [string, string, string]> = {
    fail: ['halt', '✕', 'failing'],
    pending: ['', '…', 'pending'],
    cancel: ['', '⊘', 'cancelled'],
    pass: ['live', '✓', 'passed'],
    skipping: ['done', '–', 'skipped'],
  };
  const bucketOrder = Object.keys(BUCKET);
  const REVIEW: Record<PrReviewer['state'], [string, string, string]> = {
    'changes-requested': ['halt', '✕', 'changes requested'],
    pending: ['', '…', 'review requested'],
    commented: ['acc', '●', 'commented'],
    approved: ['live', '✓', 'approved'],
    dismissed: ['done', '○', 'dismissed'],
  };
  const reviewOrder = Object.keys(REVIEW);
  const https = (u?: string) => (u && /^https:\/\//.test(u) ? u : undefined);
  const safe = $derived(https(agent.prUrl));
  const bucket = (c: PrCheck) => BUCKET[c.bucket] ?? ['', '?', c.bucket];
  const checks = $derived(
    [...(agent.prChecks ?? [])].sort((a, b) => rank(bucketOrder, a.bucket) - rank(bucketOrder, b.bucket)),
  );
  const reviewers = $derived(
    [...(agent.prReviewers ?? [])].sort((a, b) => rank(reviewOrder, a.state) - rank(reviewOrder, b.state)),
  );
  function rank(order: string[], k: string) {
    const i = order.indexOf(k);
    return i < 0 ? order.length : i;
  }
  // Agents polled before prMergeable existed only have the conflict flag.
  const merge = $derived(agent.prMergeable ?? (agent.prConflict ? 'no' : undefined));
  const open = $derived(agent.prState === 'open');
</script>

<div class="stack">
  <Panel title="Pull request">
    {#if agent.prUrl}
      <p class="wrap-any">PR: {#if safe}<a href={safe} target="_blank" rel="noopener noreferrer">{agent.prUrl}</a>{:else}<span class="mono">{agent.prUrl}</span>{/if}</p>
      {#if agent.prState}
        <p class="row">
          <span class="pill {STATE_TONE[agent.prState]}">{STATE_GLYPH[agent.prState]} {agent.prState}</span>
          {#if agent.prCi}<span class="pill {CI_TONE[agent.prCi]}">{CI_GLYPH[agent.prCi]} checks {agent.prCi}</span>{/if}
          {#if open && merge === 'no'}<span class="pill halt">✕ merge conflict</span>
          {:else if open && merge === 'yes'}<span class="pill live">✓ no conflicts</span>
          {:else if open && merge === 'unknown'}<span class="pill">? conflicts unknown</span>{/if}
          {#if agent.prPolledAt}<span class="hint">checked {agoText(agent.prPolledAt)}</span>{/if}
        </p>
      {:else if agent.status === 'watching'}
        <p class="empty">Waiting for the first check of the pull request.</p>
      {/if}
      <p class="muted">New comments on the pull request wake the agent to triage them; you approve its plan before it changes anything.</p>
    {:else}
      <p class="empty">No pull request yet. The agent opens a draft PR after you approve the ship step.</p>
    {/if}
  </Panel>
  {#if agent.prUrl && agent.prState}
    <Panel title="Checks" hint={checks.length ? `${checks.length} reported by the host` : undefined} flush={checks.length > 0}>
      {#if checks.length}
        <div class="tscroll">
          <table class="tbl">
            <thead><tr><th>Result</th><th>Check</th><th>Details</th></tr></thead>
            <tbody>
              {#each checks as c, i (i)}
                {@const [tone, glyph, label] = bucket(c)}
                <tr>
                  <td><span class="pill {tone}">{glyph} {label}</span></td>
                  <td class="wrap-any">{#if c.workflow}<span class="muted">{c.workflow} /</span> {/if}{c.name}</td>
                  <td>{#if https(c.link)}<a href={https(c.link)} target="_blank" rel="noopener noreferrer">open</a>{/if}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else}
        <p class="empty">No checks reported for this pull request.</p>
      {/if}
    </Panel>
    <Panel title="Reviewers" flush={reviewers.length > 0}>
      {#if reviewers.length}
        <ul class="list">
          {#each reviewers as r (r.name)}
            {@const [tone, glyph, label] = REVIEW[r.state]}
            <li><div class="row"><span class="pill {tone}">{glyph} {label}</span> <span class="wrap-any">{r.name}</span></div></li>
          {/each}
        </ul>
      {:else}
        <p class="empty">No reviewers asked and no reviews yet.</p>
      {/if}
    </Panel>
  {/if}
  {#if agent.prUrl}<PrComments {agent} />{/if}
</div>
