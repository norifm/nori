<script lang="ts">
  // Shuffle, previous, play or pause, next and repeat (the host's cycle: off, all, one); a jam guest's
  // shuffle and repeat are the host's. The transport's glyphs are solid, the ends lit in the accent while
  // on; a press shows a ripple.
  import { player } from "../state/player.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { big = false, ends = true }: { big?: boolean; ends?: boolean } = $props();
  const n = $derived(player.now);
  const playing = $derived(player.playing);
  /** A wait longer than 300 ms shows a spinner; before it the button shows what it will be. */
  let waiting = $state(false);
  $effect(() => {
    if (!n?.buffering) {
      waiting = false;
      return;
    }
    const t = setTimeout(() => (waiting = true), 300);
    return () => clearTimeout(t);
  });
</script>

<div class="controls" class:big>
  {#if ends && !n?.jam}
    <button class="glyph quiet side" class:on={n?.shuffle} title={W.shuffle} aria-label={W.shuffle} aria-pressed={!!n?.shuffle} onclick={() => void player.control("shuffle", !n?.shuffle)}><Icon name="shuffle" /></button>
  {/if}
  <button class="glyph skip" title={W.previous} aria-label={W.previous} onclick={() => void player.control("previous")}><Icon name="rewind-solid" /></button>
  <button class="glyph main" title={playing ? W.pause : W.play} aria-label={playing ? W.pause : W.play} onclick={() => void player.control("toggle")}>
    {#if waiting}<span class="spin"></span>{:else}{#key playing}<span class="turn"><Icon name={playing ? "pause-solid" : "play-solid"} /></span>{/key}{/if}
  </button>
  <button class="glyph skip" title={W.next} aria-label={W.next} onclick={() => void player.control("next")}><Icon name="forward-solid" /></button>
  {#if ends && !n?.jam}
    <button class="glyph quiet side" class:on={!!n?.repeat} title={n?.repeat === 1 ? W.repeatOne : W.repeat} aria-label={W.repeat} aria-pressed={!!n?.repeat} onclick={() => void player.control("cycle_repeat")}><Icon name={n?.repeat === 1 ? "repeat-once" : "repeat"} /></button>
  {/if}
</div>

<style>
  .controls {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 40px;
  }
  /* The player bar's, as the desktop's: the ends 34, skips 38 with 24 px glyphs, play 44 with 26. */
  .side {
    width: 34px;
    height: 34px;
  }
  .side :global(.icon) {
    width: 20px;
    height: 20px;
  }
  .skip {
    width: 38px;
    height: 38px;
  }
  .skip :global(.icon) {
    width: 24px;
    height: 24px;
  }
  .main {
    width: 44px;
    height: 44px;
  }
  .main :global(.icon) {
    width: 26px;
    height: 26px;
  }
  .turn {
    display: grid;
    animation: turn 180ms ease-out;
  }
  @keyframes turn {
    from {
      opacity: 0;
      transform: scale(0.8);
    }
  }
  .spin {
    position: relative;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 2px solid var(--hair);
    border-top-color: var(--ink);
    animation: spin 800ms linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  /* The full player's, as the desktop's: the ends 36 with 19 px glyphs, skips 50 with 34, play 76 with 52. */
  .big {
    gap: 20px;
    height: auto;
  }
  .big .side {
    width: 36px;
    height: 36px;
  }
  .big .side :global(.icon) {
    width: 19px;
    height: 19px;
  }
  .big .skip {
    width: 50px;
    height: 50px;
  }
  .big .skip :global(.icon) {
    width: 34px;
    height: 34px;
  }
  .big .main {
    width: 76px;
    height: 76px;
  }
  .big .main :global(.icon) {
    width: 52px;
    height: 52px;
  }
</style>
