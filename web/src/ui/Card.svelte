<script lang="ts">
  // An album's, artist's or playlist's card in a grid or shelf: its cover, title and a line under it.
  import Cover from "./Cover.svelte";

  let { href, cover, title, sub = "", round = false, fallback = "album" }: { href: string; cover?: string; title: string; sub?: string; round?: boolean; fallback?: string } = $props();
</script>

<a class="card" class:circle={round} {href}>
  <Cover src={cover} {round} {fallback} />
  <span class="title clip">{title}</span>
  {#if sub}<span class="sub clip">{sub}</span>{/if}
</a>

<style>
  .card {
    display: flex;
    flex-direction: column;
    min-width: 0;
    gap: 1px;
  }
  .card :global(.cover) {
    margin-bottom: 7px;
    transition: filter 0.15s;
  }
  .card:hover :global(.cover) {
    filter: brightness(0.92);
  }
  .title {
    font-size: 13px;
    font-weight: 500;
  }
  .circle {
    text-align: center;
  }
</style>
