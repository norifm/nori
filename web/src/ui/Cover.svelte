<script lang="ts">
  // A cover, lazily loaded, on a soft two-tone plate in the page's ink that shows while it loads; with no
  // cover the plate keeps a note (an artist's, its figure). Round for an artist; `radius` a card's or a
  // cover's corner (0 for a sleeve that runs to the edges).
  import Icon from "./Icon.svelte";

  let { src, round = false, radius = "var(--r-cover)", fallback = "music-note" }: { src?: string; round?: boolean; radius?: string; fallback?: string } = $props();
</script>

<div class="cover" style:border-radius={round ? "50%" : radius}>
  {#if src}<img {src} alt="" loading="lazy" decoding="async" />{:else}<Icon name={fallback} />{/if}
</div>

<style>
  .cover {
    position: relative;
    aspect-ratio: 1;
    width: 100%;
    overflow: hidden;
    background: var(--plate);
    display: grid;
    place-items: center;
    color: color-mix(in srgb, var(--ink) 22%, transparent);
  }
  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .cover :global(.icon) {
    width: 34%;
    height: 34%;
  }
</style>
