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

  /** `inline`: the times at the ends of the bar, in a row. */
  let { times = true, thin = false, inline = false }: { times?: boolean; thin?: boolean; inline?: boolean } = $props();

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

<div class="seek" class:thin class:inline>
  {#if times && inline}<span class="time">{W.time(shownMs)}</span>{/if}
  <div class="bar">
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
  </div>
  {#if times && inline}<span class="time">-{W.time(length - shownMs)}</span>{/if}
  {#if times && !inline && !thin}
    <div class="times"><span>{W.time(shownMs)}</span><span>-{W.time(length - shownMs)}</span></div>
  {/if}
</div>

<style>
  .seek {
    position: relative;
    width: 100%;
    padding: 6px 0 0;
  }
  .inline {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 0;
  }
  .bar {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  .track {
    height: 4px;
    border-radius: 2px;
    background: var(--hair);
    overflow: hidden;
  }
  .run {
    height: 100%;
    background: var(--ink);
    border-radius: 2px;
    transform: translateX(-100%);
    will-change: transform;
  }
  .thin .track {
    height: 2px;
    border-radius: 0;
  }
  input {
    position: absolute;
    left: 0;
    right: 0;
    top: -9px;
    width: 100%;
    height: 22px;
    margin: 0;
    opacity: 0;
    cursor: pointer;
  }
  .time,
  .times {
    font-size: 11px;
    font-weight: 500;
    color: var(--ink2);
    font-variant-numeric: tabular-nums;
  }
  .time {
    min-width: 34px;
  }
  .time:last-child {
    text-align: right;
  }
  .times {
    display: flex;
    justify-content: space-between;
    padding-top: 6px;
  }
</style>
