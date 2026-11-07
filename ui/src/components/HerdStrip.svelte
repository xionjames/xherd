<script lang="ts">
  // Every agent as a pill: "Needs you N", the waiting ones (oldest first), then the rest.
  import AgentDot from './AgentDot.svelte';
  import { app } from '../lib/store.svelte';
  import { href } from '../lib/router.svelte';
  import { ago, needsYou, waitLabel, STATUS_LABEL } from '../lib/format';
  let { current }: { current?: string } = $props();

  const agents = $derived(app.state?.agents ?? []);
  const waiting = $derived(agents.filter(needsYou).sort((a, b) => a.statusSince - b.statusSince));
  const rest = $derived(agents.filter((a) => !needsYou(a)).sort((a, b) => b.lastActivity - a.lastActivity));
</script>

{#if agents.length}
  <nav class="herd" aria-label="Agents">
    {#if waiting.length}
      <span class="lbl" style="color:var(--gate);white-space:nowrap;flex:none">Needs you {waiting.length}</span>
      {#each waiting as a (a.id)}
        <a class="hc need" class:on={current === a.id} href={href(`/agent/${a.name}`)}
           aria-current={current === a.id ? 'page' : undefined}>
          <AgentDot agent={a} />{a.name}<span class="s">{waitLabel(a)} · {ago(a.statusSince)}</span>
        </a>
      {/each}
      {#if rest.length}<span class="hsep"></span>{/if}
    {/if}
    {#each rest as a (a.id)}
      <a class="hc" class:on={current === a.id} href={href(`/agent/${a.name}`)}
         aria-current={current === a.id ? 'page' : undefined}>
        <AgentDot agent={a} />{a.name}<span class="s">{STATUS_LABEL[a.status]}</span>
      </a>
    {/each}
  </nav>
{/if}
