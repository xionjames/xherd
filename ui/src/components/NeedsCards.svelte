<script lang="ts">
  // Agents waiting for you, oldest first: what waits and the button that opens
  // the agent tab holding the decision. Used by the Herd and Needs attention.
  import AgentDot from './AgentDot.svelte';
  import { href, go } from '../lib/router.svelte';
  import { ago, repoName, waitText } from '../lib/format';
  import type { Agent } from '../lib/types';

  let { agents }: { agents: Agent[] } = $props();

  /** The tab that holds the decision for a waiting agent. */
  function decisionTab(a: Agent): string {
    const g = a.gate;
    if (!g) return 'terminal';
    if (g.kind === 'commit') return 'changes';
    if (g.kind === 'question') return 'terminal';
    if (g.kind === 'docs' && g.role === 'ship') return 'changes';
    if (g.kind === 'docs') return 'spec';
    return 'pr';
  }
  function actionLabel(a: Agent): string {
    const g = a.gate;
    if (!g || g.kind === 'question') return 'Answer';
    if (g.kind === 'commit') return 'Review changes';
    if (g.kind === 'docs') return 'Review';
    return 'Decide';
  }
  function meta(a: Agent): string {
    const g = a.gate;
    const parts = [repoName(a)];
    if (g?.kind === 'commit') parts.push(`${g.files.length} files`);
    if (g?.kind === 'docs' && g.questions?.length) parts.push(`${g.questions.length} open questions`);
    if (g && 'needsHuman' in g && g.needsHuman) parts.push(`held: ${g.needsHuman}`);
    if (!g) parts.push('asked in the terminal');
    return parts.join(' · ');
  }
</script>

<div class="nygrid">
  {#each agents as a (a.id)}
    <article class="card need cb tight">
      <div class="row">
        <AgentDot agent={a} /><a class="mono nyname" href={href(`/agent/${a.name}`)}>{a.name}</a>
        <span class="grow"></span><span class="muted num">{ago(a.statusSince)}</span>
      </div>
      <p class="wrap-any" class:mono={a.gate?.kind === 'commit'}>{waitText(a)}</p>
      <p class="hint">{meta(a)}</p>
      <div class="row end">
        <button class="btn gate" onclick={() => go(`/agent/${a.name}`, { s: decisionTab(a) })}>{actionLabel(a)}</button>
      </div>
    </article>
  {/each}
</div>

<style>
  .nygrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 340px), 1fr)); gap: 14px; }
  .nyname { font-weight: 600; color: var(--ink); }
</style>
