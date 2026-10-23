<script lang="ts">
  // An album's, artist's or playlist's card in a grid or shelf: its cover, title and a line under it.
  import Cover from "./Cover.svelte";

  let { href, cover, title, sub = "", round = false, fallback = "compact-disc" }: { href: string; cover?: string; title: string; sub?: string; round?: boolean; fallback?: string } = $props();
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
    transition: transform 0.15s;
  }
  .card:hover :global(.cover) {
    transform: scale(1.02);
  }
  .title {
    font-size: 13.5px;
    font-weight: 600;
  }
  .sub {
    font-size: 12.5px;
  }
  .circle {
    text-align: center;
  }
</style>
