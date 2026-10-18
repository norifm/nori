<script lang="ts">
  // Shuffle, previous, play or pause, next and repeat (the host's cycle: off, all, one); a jam guest's
  // shuffle and repeat are the host's.
  import { player } from "../state/player.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { big = false, ends = true }: { big?: boolean; ends?: boolean } = $props();
  const n = $derived(player.now);
  const playing = $derived(player.playing);
</script>

<div class="controls" class:big>
  {#if ends && !n?.jam}
    <button class="round side" class:on={n?.shuffle} title={W.shuffle} aria-label={W.shuffle} aria-pressed={!!n?.shuffle} onclick={() => void player.control("shuffle", !n?.shuffle)}><Icon name="shuffle" /></button>
  {/if}
  <button class="round skip" title={W.previous} aria-label={W.previous} onclick={() => void player.control("previous")}><Icon name="previous" /></button>
  <button class="round main" title={playing ? W.pause : W.play} aria-label={playing ? W.pause : W.play} onclick={() => void player.control("toggle")}><Icon name={playing ? "pause" : "play"} /></button>
  <button class="round skip" title={W.next} aria-label={W.next} onclick={() => void player.control("next")}><Icon name="next" /></button>
  {#if ends && !n?.jam}
    <button class="round side" class:on={!!n?.repeat} title={n?.repeat === 1 ? W.repeatOne : W.repeat} aria-label={W.repeat} aria-pressed={!!n?.repeat} onclick={() => void player.control("cycle_repeat")}><Icon name={n?.repeat === 1 ? "repeat-one" : "repeat"} /></button>
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
    color: var(--dim);
  }
  .side.on {
    color: var(--accent);
  }
  .main {
    width: 40px;
    height: 40px;
  }
  .main :global(.icon) {
    width: 28px;
    height: 28px;
  }
  .skip :global(.icon) {
    width: 22px;
    height: 22px;
  }
  .big {
    gap: 28px;
  }
  .big .main {
    width: 64px;
    height: 64px;
  }
  .big .main :global(.icon) {
    width: 46px;
    height: 46px;
  }
  .big .skip {
    width: 48px;
    height: 48px;
  }
  .big .skip :global(.icon) {
    width: 34px;
    height: 34px;
  }
</style>
