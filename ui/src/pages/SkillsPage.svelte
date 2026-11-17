<script lang="ts">
  // Team → Skills: the library agents draw from (it used to be a Settings tab).
  import PageHead from '../components/PageHead.svelte';
  import TeamTabs from '../components/TeamTabs.svelte';
  import Skills from '../components/Skills.svelte';
</script>

<PageHead width="mid" title="Team" sub="The skills agents can be given: a folder with a SKILL.md, copied into a job's worktree when it starts.">
  {#snippet tabs()}<TeamTabs current="skills" />{/snippet}
</PageHead>
<div class="stack mid"><Skills /></div>
