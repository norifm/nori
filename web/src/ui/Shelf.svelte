<script lang="ts">
  // A row of `count` cards that scrolls sideways, as the desktop's shelf: on a wide window as many whole
  // cards as fit between the gutters (two at the fewest, each at least `least` wide, grown to reach both),
  // seen from a gap before the left gutter to a gap past the right one so the cards beside the whole ones
  // are not glimpsed, at rest on a whole card, and paged by the arrows over its ends while the pointer is
  // on it. On a phone the cards keep their size and run out to the window's edges, as Android's.
  import type { Snippet } from "svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { count, least = 150, children }: { count: number; least?: number; children: Snippet } = $props();

  let row = $state<HTMLElement>();
  /** The row's width, a gap past each gutter. */
  let width = $state(0);
  let gap = $state(0);
  let left = $state(0);
  $effect(() => {
    if (row) gap = parseFloat(getComputedStyle(row).columnGap);
  });
  const room = $derived(width - 2 * gap);
  const columns = $derived(Math.max(2, Math.floor((room + gap) / (least + gap))));
  const side = $derived((room - (columns - 1) * gap) / columns);
  const end = $derived(Math.max(0, count - columns) * (side + gap));
  const still = () => document.documentElement.classList.contains("still");
  const page = (by: number) => row?.scrollBy({ left: by * columns * (side + gap), behavior: still() ? "instant" : "smooth" });
</script>

<div class="shelf-cards" class:whole={ui.wide} style:--side={ui.wide ? `${side}px` : null}>
  <div class="row" bind:this={row} bind:clientWidth={width} onscroll={() => (left = row?.scrollLeft ?? 0)}>{@render children()}</div>
  {#if ui.wide}
    <button class="arrow back" class:shown={left > 1} tabindex="-1" aria-label={W.previous} onclick={() => page(-1)}><Icon name="nav-arrow-left" /></button>
    <button class="arrow on" class:shown={left < end - 1} tabindex="-1" aria-label={W.next} onclick={() => page(1)}><Icon name="nav-arrow-right" /></button>
  {/if}
</div>

<style>
  .shelf-cards {
    position: relative;
  }
  .row {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 148px;
    gap: var(--grid-gap);
    overflow-x: auto;
    scroll-snap-type: x mandatory;
    margin: 0 calc(-1 * var(--gutter));
    padding: 0 var(--gutter) 6px;
    scroll-padding: 0 var(--gutter);
    scrollbar-width: none;
  }
  .whole .row {
    grid-auto-columns: var(--side);
    margin: 0 calc(-1 * var(--grid-gap));
    padding: 0 var(--grid-gap) 6px;
    scroll-padding: 0 var(--grid-gap);
  }
  .row::-webkit-scrollbar {
    display: none;
  }
  .row > :global(*) {
    scroll-snap-align: start;
  }
  /* Over the gutters, level with the middle of a card's square. */
  .arrow {
    position: absolute;
    top: calc(var(--side) / 2 - 17px);
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    border: 1px solid var(--outline);
    background: color-mix(in srgb, var(--page) 85%, transparent);
    color: var(--ink);
    opacity: 0;
    pointer-events: none;
    transition: opacity 160ms ease-out;
  }
  .arrow:hover {
    background: var(--surface);
  }
  .arrow :global(.icon) {
    width: 16px;
    height: 16px;
  }
  .back {
    left: -17px;
  }
  .on {
    right: -17px;
  }
  .shelf-cards:hover .arrow.shown {
    opacity: 1;
    pointer-events: auto;
  }
</style>
