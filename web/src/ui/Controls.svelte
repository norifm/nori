<script lang="ts">
  // Previous, play or pause, and next: the transport's solid glyphs, bare, each pressed with a ripple. A
  // jam guest's are what its role offers (the player decides). `big`: the full player's sizes.
  import { player } from "../state/player.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { big = false }: { big?: boolean } = $props();
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

<button class="glyph skip" class:big title={W.previous} aria-label={W.previous} onclick={() => void player.control("previous")}><Icon name="rewind-solid" /></button>
<button class="glyph main" class:big title={playing ? W.pause : W.play} aria-label={playing ? W.pause : W.play} onclick={() => void player.control("toggle")}>
  {#if waiting}<span class="spin"></span>{:else}{#key playing}<span class="turn"><Icon name={playing ? "pause-solid" : "play-solid"} /></span>{/key}{/if}
</button>
<button class="glyph skip" class:big title={W.next} aria-label={W.next} onclick={() => void player.control("next")}><Icon name="forward-solid" /></button>

<style>
  .skip :global(.icon) {
    width: 26px;
    height: 26px;
  }
  .main :global(.icon) {
    width: 32px;
    height: 32px;
  }
  .skip.big {
    width: 56px;
    height: 56px;
  }
  .skip.big :global(.icon) {
    width: 38px;
    height: 38px;
  }
  .main.big {
    width: 68px;
    height: 68px;
  }
  .main.big :global(.icon) {
    width: 60px;
    height: 60px;
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
  .big .spin {
    width: 26px;
    height: 26px;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
