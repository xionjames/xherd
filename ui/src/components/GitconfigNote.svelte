<script lang="ts">
  // Native runs only: xherd's git settings live in its own gitconfig, which the
  // user's ~/.gitconfig has to include. Shown above the identity and the hosts.
  import { get } from '../lib/api';
  import Callout from './Callout.svelte';
  import type { GitHostsView } from '../lib/types';
  let gitconfig = $state('');
  $effect(() => {
    get<GitHostsView>('/git/hosts').then((v) => { gitconfig = !v.sandbox && v.gitconfig ? v.gitconfig : ''; }).catch(() => {});
  });
</script>

{#if gitconfig}
  <Callout tone="info">xherd runs natively, so these settings go to <span class="mono">{gitconfig}</span>. To use them, add
    <span class="mono" style="user-select:all">[include] path = {gitconfig}</span> to <span class="mono">~/.gitconfig</span>.
    Hosts shown as coming from a Git provider plugin are not written there, so your own git setup for them stays as it is;
    add the host as a row (Edit) for xherd to set its credential helper.</Callout>
{/if}
