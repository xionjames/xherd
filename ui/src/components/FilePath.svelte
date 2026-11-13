<script lang="ts">
  // A file path on one line that fits its width: when it is too long its start
  // gives way (`…/very/deep/MyFile.html`, then `…yFile.html`), so the name and
  // the nearest folders stay. The full path is the tooltip, and a copy takes it all.
  let { path }: { path: string } = $props();
</script>

<span class="fp mono" title={path}><bdi>{path}</bdi></span>

<style>
  /* Right-to-left overflow cuts the start; the text itself stays left-to-right. */
  .fp { display: block; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; direction: rtl; text-align: left; }
  .fp bdi { direction: ltr; unicode-bidi: isolate; }
</style>
