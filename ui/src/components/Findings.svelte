<script lang="ts">
  // Findings of `review.md` with a colour per severity (R-114). The colour is
  // never the only signal: each severity also has a glyph and its name.
  import type { Agent, Finding, Severity } from '../lib/types';
  import { get } from '../lib/api';

  let { agent }: { agent: Agent } = $props();
  // Effects key on the id: the agent object is replaced on every update.
  const id = $derived(agent.id);
  // Reload when the document or the tasks that fix findings change.
  const key = $derived(
    (agent.gate?.kind === 'docs' ? agent.gate.files.map((f) => f.sha256).join() : '') +
      (agent.tasks ?? []).map((t) => t.status).join(),
  );

  const GLYPH: Record<Severity, string> = { blocker: '■', major: '▲', minor: '●' };
  const ORDER: Severity[] = ['blocker', 'major', 'minor'];

  let findings = $state<Finding[]>([]);
  let error = $state<string | null>(null);

  $effect(() => {
    void key;
    get<{ findings: Finding[]; error?: string }>(`/agents/${id}/findings`).then(
      (r) => { findings = r.findings; error = r.error ?? null; },
      (e) => { findings = []; error = e.message; },
    );
  });

  const sorted = $derived([...findings].sort((a, b) => ORDER.indexOf(a.severity) - ORDER.indexOf(b.severity)));
  const count = (s: Severity) => findings.filter((f) => f.severity === s && !f.resolved).length;

  function stateText(f: Finding): string {
    if (f.resolved) return `fixed by ${f.fixed?.join(', ')} ✓`;
    if (f.fixed?.length) return `waiting for ${f.fixed.join(', ')}`;
    if (f.left) return `left: ${f.left}`;
    return 'not fixed';
  }
</script>

{#if error}
  <div class="card cb" style="color:var(--sev-blocker)">review.md: {error}</div>
{:else if findings.length}
  <section class="card" aria-label="Review findings">
    <div class="ch">
      <h2>Findings</h2>
      <span class="row">
        {#each ORDER as s (s)}
          {#if count(s)}<span class="pill sev-{s}">{GLYPH[s]} {count(s)} {s}</span>{/if}
        {/each}
        {#if !findings.some((f) => !f.resolved)}<span class="pill live">✓ all fixed</span>{/if}
      </span>
    </div>
    <ul class="list">
      {#each sorted as f (f.id)}
        <li class="item sev-{f.severity}" class:done={f.resolved}>
          <span class="pill sev-{f.severity}">{GLYPH[f.severity]} {f.severity}</span>
          <span class="mono muted">{f.id}</span>
          <span class="grow">{f.title}</span>
          {#if f.location}<span class="mono muted">{f.location}</span>{/if}
          <span class="muted state">{stateText(f)}</span>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .list { list-style: none; margin: 0; padding: 0; }
  .item { display: flex; align-items: center; gap: 10px; padding: 8px 14px; border-top: 1px solid var(--fence-soft); box-shadow: inset 3px 0 0 var(--sev-minor); flex-wrap: wrap; }
  .item.sev-blocker { box-shadow: inset 3px 0 0 var(--sev-blocker); }
  .item.sev-major { box-shadow: inset 3px 0 0 var(--sev-major); }
  .item.done { opacity: .6; }
  .state { font-size: 12px; }
</style>
