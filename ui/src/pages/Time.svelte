<script lang="ts">
  // Work log (spec §16.4): agent time and your time per day and ticket, and
  // per-ticket delivery metrics measured from what happened.
  import { get } from '../lib/api';
  import { href } from '../lib/router.svelte';
  import { span } from '../lib/format';
  import { showError } from '../lib/errors.svelte';
  import PageHead from '../components/PageHead.svelte';
  import Panel from '../components/Panel.svelte';

  type Day = { day: string; tickets: { ticket: string; agentMs: number; userMs: number }[] };
  type Metric = { ticket: string; agent: string; status: string; cycleMs: number | null; reviewRounds: number; autoApprovals: number; rework: number };
  let data = $state<{ days: Day[]; tickets: Metric[] } | null>(null);
  let days = $state('14');
  $effect(() => {
    const n = days;
    get<{ days: Day[]; tickets: Metric[] }>(`/worklog?days=${n}`).then((r) => (data = r)).catch((e) => showError('Could not load the work log', e));
  });
  const total = (d: Day, k: 'agentMs' | 'userMs') => d.tickets.reduce((n, t) => n + t[k], 0);
</script>

<PageHead title="Time" sub="Agent time is time spent running; your time is time with an agent's page open and in front. Days are UTC.">
  {#snippet actions()}
    <select aria-label="Period" bind:value={days}>
      <option value="7">Last 7 days</option><option value="14">Last 14 days</option><option value="30">Last 30 days</option><option value="90">Last 90 days</option>
    </select>
  {/snippet}
</PageHead>

<div class="stack mid">
  {#if !data}
    <p class="empty">Loading…</p>
  {:else}
    <div class="sect"><h2>Per day</h2></div>
    {#if !data.days.length}<p class="empty">Nothing recorded in this period.</p>{/if}
    {#each data.days as d (d.day)}
      <Panel label={d.day} flush>
        {#snippet head()}
          <h2 class="mono grow">{d.day}</h2>
          <span class="hint">agents {span(total(d, 'agentMs'))} · you {span(total(d, 'userMs'))}</span>
        {/snippet}
        <div class="tscroll">
          <table class="tbl">
            <tbody>
              {#each d.tickets as t (t.ticket)}
                <tr><td class="mono">{t.ticket}</td><td class="num r">agent {span(t.agentMs)}</td><td class="num r">you {span(t.userMs)}</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
      </Panel>
    {/each}
    <Panel title="Delivery per ticket" hint="Cycle time runs from creating the agent to the pull request merged or closed. Rework counts auto-approvals you later asked changes on, on the same gate." flush>
      {#if !data.tickets.length}<div class="cb"><p class="empty">No agents yet.</p></div>{:else}
        <div class="tscroll">
          <table class="tbl">
            <thead><tr><th>Ticket</th><th>Agent</th><th class="r">Cycle time</th><th class="r">Review rounds</th><th class="r">Auto-approved</th><th class="r">Rework</th></tr></thead>
            <tbody>
              {#each data.tickets as t (t.agent)}
                <tr>
                  <td class="mono">{t.ticket}</td>
                  <td><a href={href(`/agent/${t.agent}`)}>{t.agent}</a></td>
                  <td class="num r">{t.cycleMs === null ? 'open' : span(t.cycleMs)}</td>
                  <td class="num r">{t.reviewRounds}</td>
                  <td class="num r">{t.autoApprovals}</td>
                  <td class="num r">{t.rework}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </Panel>
  {/if}
</div>
