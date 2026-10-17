<script lang="ts">
  // A cover, lazily loaded; a plain square while there is none (round for an artist).
  import Icon from "./Icon.svelte";

  let { src, round = false, fallback = "note" }: { src?: string; round?: boolean; fallback?: string } = $props();
</script>

<div class="cover" class:round>
  {#if src}<img {src} alt="" loading="lazy" decoding="async" />{:else}<Icon name={fallback} />{/if}
</div>

<style>
  .cover {
    position: relative;
    aspect-ratio: 1;
    width: 100%;
    border-radius: 6px;
    overflow: hidden;
    background: var(--cover-ground);
    display: grid;
    place-items: center;
    color: var(--faint);
    box-shadow: inset 0 0 0 0.5px var(--hair);
  }
  .round {
    border-radius: 50%;
  }
  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .cover :global(.icon) {
    width: 36%;
    height: 36%;
  }
</style>
