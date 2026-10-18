<script lang="ts" module>
  import { facts } from "../rpc";

  /** Second timers pending across the seek bars: none while nothing plays or the page is hidden. */
  let pending = 0;
  const count = (d: number) => (facts.clockTimers = pending += d);
</script>

<script lang="ts">
  // The seek bar. The bar runs on by itself while the song plays (an animation the browser draws, from
  // where the song is to its end at its pace); the times move each second while the page is seen.
  // Nothing runs while the song holds or the page is hidden.
  import { player } from "../state/player.svelte";
  import { W } from "../words";

  let { times = true, thin = false }: { times?: boolean; thin?: boolean } = $props();

  let run = $state<HTMLDivElement>();
  let dragging = $state(false);
  let dragMs = $state(0);
  /** The position the times show. */
  let at = $state(0);
  const length = $derived(player.anchor.length);
  const shownMs = $derived(dragging ? dragMs : at);

  /** The bar's place for `ms`: shifted left by what is still to play. */
  const shift = (ms: number) => `translateX(${length ? (Math.min(ms, length) / length - 1) * 100 : -100}%)`;

  $effect(() => {
    const a = player.anchor;
    if (!run) return;
    for (const x of run.getAnimations()) x.cancel();
    if (dragging) {
      run.style.transform = shift(dragMs);
      return;
    }
    const ms = player.position();
    run.style.transform = shift(ms);
    if (a.pace > 0 && a.length > ms) run.animate([{ transform: shift(ms) }, { transform: shift(a.length) }], { duration: (a.length - ms) / a.pace, easing: "linear", fill: "forwards" });
  });

  $effect(() => {
    if (!times) return;
    const a = player.anchor;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const stop = () => {
      if (timer === undefined) return;
      clearTimeout(timer);
      timer = undefined;
      count(-1);
    };
    const tick = () => {
      if (timer !== undefined) {
        timer = undefined;
        count(-1);
      }
      const ms = player.position();
      at = ms;
      if (a.pace > 0 && !document.hidden && ms < a.length) {
        timer = setTimeout(tick, (1000 - (ms % 1000)) / a.pace + 5);
        count(1);
      }
    };
    const seen = () => (stop(), tick());
    tick();
    document.addEventListener("visibilitychange", seen);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", seen);
    };
  });
</script>

<div class="seek" class:thin>
  <div class="track"><div class="run" bind:this={run}></div></div>
  {#if !thin}
    <input
      type="range"
      min="0"
      max={Math.max(1, length)}
      step="1000"
      value={shownMs}
      aria-label={W.seek}
      oninput={(e) => {
        dragging = true;
        dragMs = Number(e.currentTarget.value);
      }}
      onchange={(e) => {
        const ms = Number(e.currentTarget.value);
        at = ms;
        dragging = false;
        void player.seek(ms);
      }}
    />
  {/if}
  {#if times && !thin}
    <div class="times"><span class="at">{W.time(shownMs)}</span><span>-{W.time(length - shownMs)}</span></div>
  {/if}
</div>

<style>
  .seek {
    position: relative;
    width: 100%;
    padding: 6px 0 0;
  }
  .track {
    height: 4px;
    border-radius: 2px;
    background: var(--fill);
    overflow: hidden;
    transition: height 0.12s;
  }
  .run {
    height: 100%;
    background: var(--dim);
    border-radius: 2px;
    transform: translateX(-100%);
    will-change: transform;
  }
  .seek:hover .track {
    height: 6px;
  }
  .seek:hover .run {
    background: var(--text);
  }
  .thin {
    padding: 0;
  }
  .thin .track {
    height: 2px;
    border-radius: 0;
  }
  input {
    position: absolute;
    left: 0;
    right: 0;
    top: -4px;
    width: 100%;
    height: 22px;
    margin: 0;
    opacity: 0;
    cursor: pointer;
  }
  .times {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    font-weight: 500;
    color: var(--faint);
    font-variant-numeric: tabular-nums;
    padding-top: 4px;
  }
</style>
