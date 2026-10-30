<script lang="ts">
  // A section's heading on the gutter. With `href` the whole heading opens the section in full, a quiet
  // arrow at its end; with `toggle` it opens the section whole in place (`open`, the arrow turned down) and
  // folds it again.
  import Icon from "./Icon.svelte";

  let { title, href, toggle, open = false }: { title: string; href?: string; toggle?: () => void; open?: boolean } = $props();
</script>

{#if href}
  <a class="section-title" {href}><span>{title}</span><Icon name="nav-arrow-right" /></a>
{:else if toggle}
  <button class="section-title" aria-expanded={open} onclick={toggle}><span>{title}</span><Icon name={open ? "nav-arrow-down" : "nav-arrow-right"} /></button>
{:else}
  <h2 class="section-title"><span>{title}</span></h2>
{/if}
