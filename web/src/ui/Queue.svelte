<script lang="ts">
  // The queue in play order: what played (dimmed), the song heard, what plays next. A click plays a
  // song; its menu and its handle's drag move or remove it (dragging only while play order is list order).
  import { player } from "../state/player.svelte";
  import { songMenu } from "../state/menus";
  import { W } from "../words";
  import type { QueueSong, Song } from "../state/types";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import JamQueue from "./JamQueue.svelte";
  import { sortable } from "./sortable";

  const n = $derived(player.now);
  const rows = $derived(n?.queue ?? []);
  const current = $derived(n?.current ?? -1);
  let list = $state<HTMLOListElement>();
  let placed = false;
  // Opened with the song heard at the top and what played above it, as Android's queue opens; once.
  $effect(() => {
    const el = list?.querySelector<HTMLElement>(".current");
    if (placed || !list || !el) return;
    placed = true;
    list.scrollTop += el.getBoundingClientRect().top - list.getBoundingClientRect().top;
  });
  const move = (from: number, to: number) => player.control("move_song", from, to);
  const extra = (q: QueueSong, at: number) => [
    ...(n?.reorderable && at > 0 ? [{ label: W.moveUp, icon: "nav-arrow-up", run: () => move(q.index, rows[at - 1].index) }] : []),
    ...(n?.reorderable && at < rows.length - 1 ? [{ label: W.moveDown, icon: "nav-arrow-down", run: () => move(q.index, rows[at + 1].index) }] : []),
    ...(at !== current ? [{ label: W.remove, icon: "xmark", danger: true, run: () => player.control("remove", q.index) }] : []),
  ];
</script>

<JamQueue />
<ol class="queue" id="queue" bind:this={list} use:sortable={{ moved: (from, to) => move(rows[from].index, rows[to].index) }}>
  {#each rows as q, at (`${q.index}:${q.id}`)}
    {#if at === 0 && current > 0}<li class="head">{W.history}</li>{/if}
    {#if at === current + 1}<li class="head">{W.queue}</li>{/if}
    <li class="row" class:current={at === current} class:past={at < current} data-row={n?.reorderable && at !== current ? "" : undefined} data-at={at}>
      <button class="play" onclick={() => void player.control("jump", q.index)}>
        <span class="art"><Cover src={q.cover} /></span>
        <span class="text">
          <span class="title clip">{q.title ?? q.id ?? ""}</span>
          <span class="sub clip">{q.by ? W.jamAddedBy(q.by) : (q.artist ?? "")}</span>
        </span>
        {#if at === current}<Icon name={player.playing ? "play-solid" : "pause-solid"} />{/if}
      </button>
      {#if n?.reorderable && at !== current}
        <span class="handle" data-handle aria-label={W.reorder} title={W.reorder}><Icon name="menu" /></span>
      {/if}
      {#if q.id}
        <button class="round more" aria-label={W.more} title={W.more} onclick={() => void songMenu(q as Song, extra(q, at))}><Icon name="more-horiz" /></button>
      {/if}
    </li>
  {:else}
    <li class="none">{W.nothingPlaying}</li>
  {/each}
</ol>

<style>
  .queue {
    list-style: none;
    margin: 0;
    padding: 0 0 24px;
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }
  .head {
    font-size: 15px;
    font-weight: 700;
    letter-spacing: -0.2px;
    padding: 18px 8px 8px;
  }
  .row {
    display: flex;
    align-items: center;
    border-radius: 12px;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.current .title {
    color: var(--accent);
    font-weight: 600;
  }
  .row.past {
    opacity: 0.5;
  }
  .row:global(.held) {
    position: relative;
    z-index: 1;
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--outline);
  }
  .handle {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    color: var(--dim);
    cursor: grab;
    touch-action: none;
  }
  .play {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px;
    text-align: left;
  }
  .play > :global(.icon) {
    color: var(--accent);
    width: 16px;
    height: 16px;
  }
  .art {
    width: 40px;
    flex: none;
  }
  .art :global(.cover) {
    border-radius: 7px;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title {
    font-size: 14px;
  }
  .more {
    color: var(--ink2);
    opacity: 0;
  }
  .row:hover .more,
  .more:focus-visible {
    opacity: 1;
  }
  @media (hover: none) {
    .more {
      opacity: 1;
    }
  }
  .none {
    color: var(--ink2);
    padding: 24px 8px;
  }
</style>
