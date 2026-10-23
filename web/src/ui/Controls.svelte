<script lang="ts">
  // Shuffle, previous, play or pause, next and repeat (the host's cycle: off, all, one); a jam guest's
  // shuffle and repeat are the host's. Play and pause are the bare glyph; the press shows a ripple.
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
    <button class="round side" class:on={n?.shuffle} title={W.shuffle} aria-label={W.shuffle} aria-pressed={!!n?.shuffle} onclick={() => void player.control("shuffle", !n?.shuffle)}><Icon name="shuffle" /></button>
  {/if}
  <button class="round skip" title={W.previous} aria-label={W.previous} onclick={() => void player.control("previous")}><Icon name="rewind-solid" /></button>
  <button class="round main" title={playing ? W.pause : W.play} aria-label={playing ? W.pause : W.play} onclick={() => void player.control("toggle")}>
    {#if waiting}<span class="spin"></span>{:else}{#key playing}<span class="glyph"><Icon name={playing ? "pause-solid" : "play-solid"} /></span>{/key}{/if}
  </button>
  <button class="round skip" title={W.next} aria-label={W.next} onclick={() => void player.control("next")}><Icon name="forward-solid" /></button>
  {#if ends && !n?.jam}
    <button class="round side" class:on={!!n?.repeat} title={n?.repeat === 1 ? W.repeatOne : W.repeat} aria-label={W.repeat} aria-pressed={!!n?.repeat} onclick={() => void player.control("cycle_repeat")}><Icon name={n?.repeat === 1 ? "repeat-once" : "repeat"} /></button>
  {/if}
</div>

<style>
  .controls {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
  }
  .side {
    color: var(--ink2);
  }
  .side.on {
    color: var(--accent);
  }
  .skip {
    color: var(--ink);
  }
  .main {
    width: 46px;
    height: 46px;
  }
  .main :global(.icon) {
    width: 30px;
    height: 30px;
  }
  .glyph {
    display: grid;
    animation: glyph 180ms ease-out;
  }
  @keyframes glyph {
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
  .big {
    gap: 22px;
  }
  .big .round {
    width: 52px;
    height: 52px;
  }
  .big .round :global(.icon) {
    width: 28px;
    height: 28px;
  }
  .big .main {
    width: 84px;
    height: 84px;
  }
  .big .main :global(.icon) {
    width: 56px;
    height: 56px;
  }
</style>
