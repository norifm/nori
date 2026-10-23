<script lang="ts">
  // A section's heading on the gutter. With `href` the whole heading opens the section in full, a quiet
  // arrow at its end.
  import Icon from "./Icon.svelte";

  let { title, href }: { title: string; href?: string } = $props();
</script>

{#if href}
  <a class="section-title" {href}><span>{title}</span><Icon name="nav-arrow-right" /></a>
{:else}
  <h2 class="section-title"><span>{title}</span></h2>
{/if}
