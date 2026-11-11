<script lang="ts" module>
  export type Tab = { id: string; label: string; href: string; badge?: { n: string | number; tone?: '' | 'g' | 'h' } };
</script>

<script lang="ts">
  // Page or section tabs: links, so each tab has its own URL. Scrolls
  // sideways on a narrow screen.
  let { items, current, label }: { items: Tab[]; current: string; label: string } = $props();
</script>

<nav class="tabs" aria-label={label}>
  {#each items as t (t.id)}
    <a class="tb" class:on={t.id === current} aria-current={t.id === current ? 'page' : undefined} href={t.href}>
      {t.label}{#if t.badge}<span class="tc {t.badge.tone ?? ''}">{t.badge.n}</span>{/if}
    </a>
  {/each}
</nav>
