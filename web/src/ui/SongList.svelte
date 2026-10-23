<script lang="ts">
  // Songs as rows on one plate: a click plays them from there, as the page's list (`kind`, `id`); each has
  // its menu (`extra`: the page's own entries). `numbered` shows track numbers instead of covers (an
  // album's); `rows` the positions shown (a disc's); `shown` only the first few, with Show all under them;
  // `moved`, each row's handle drags it to another place (a playlist's); `one`, a click plays that song
  // alone (a search's results).
  import { player } from "../state/player.svelte";
  import { songMenu } from "../state/menus";
  import type { Item } from "../state/ui.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import { sortable } from "./sortable";

  let {
    songs,
    kind = "",
    id = "",
    numbered = false,
    rows,
    shown,
    moved,
    one = false,
    extra = () => [],
  }: { songs: Song[]; kind?: string; id?: string; numbered?: boolean; rows?: number[]; shown?: number; moved?: (from: number, to: number) => unknown; one?: boolean; extra?: (s: Song, i: number) => Item[] } = $props();
  const playingId = $derived(player.song?.id);
  let all = $state(false);
  const at = $derived(rows ?? songs.map((_, i) => i));
  const drawn = $derived(all || shown === undefined ? at : at.slice(0, shown));
</script>

<ol class="songs" class:numbered use:sortable={{ moved: (from, to) => moved?.(from, to) }}>
  {#each drawn as i (`${i}:${songs[i].id}`)}
    {@const s = songs[i]}
    <li class:now={s.id === playingId} data-row={moved ? "" : undefined} data-at={i}>
      <button class="play" onclick={() => void (one ? player.play([s], 0, false, kind, id) : player.play(songs, i, false, kind, id))}>
        {#if numbered}
          <span class="n">{#if s.id === playingId}<span class="bars" class:still={!player.playing}><i></i><i></i><i></i></span>{:else}{s.track || i + 1}{/if}</span>
        {:else}
          <span class="art"><Cover src={s.cover} /></span>
        {/if}
        <span class="title clip">{s.title}</span>
        <span class="artist clip">{s.artist}</span>
        {#if !numbered}<span class="album clip">{s.album}</span>{/if}
      </button>
      {#if s.starred}<span class="starred" aria-label={W.favorite}><Icon name="sealed" /></span>{/if}
      <span class="len">{W.time(s.duration * 1000)}</span>
      <button class="more" aria-label={W.more} title={W.more} onclick={() => void songMenu(s, extra(s, i))}><Icon name="more-horiz" /></button>
      {#if moved}<span class="handle" data-handle aria-label={W.reorder} title={W.reorder}><Icon name="menu" /></span>{/if}
    </li>
  {/each}
</ol>
{#if drawn.length < at.length}<button class="text-action all" onclick={() => (all = true)}>{W.showAll}</button>{/if}

<style>
  .all {
    margin-top: 8px;
  }
  .songs {
    list-style: none;
    margin: 0;
    padding: 0;
    background: var(--surface);
    border: var(--hairline) solid var(--hair);
    border-radius: var(--r-tile);
    overflow: hidden;
  }
  li {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    padding-right: 8px;
  }
  /* The hairline starts where the title does. */
  li::after {
    content: "";
    position: absolute;
    left: 74px;
    right: 0;
    bottom: 0;
    height: 1px;
    background: var(--line);
  }
  .numbered li::after {
    left: 60px;
  }
  li:last-child::after {
    display: none;
  }
  li:hover {
    background: var(--hover);
  }
  li:global(.held) {
    z-index: 1;
    background: var(--surface);
    box-shadow: 0 0 0 var(--hairline) var(--hair);
  }
  .handle {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    color: var(--ink2);
    cursor: grab;
    touch-action: none;
  }
  .handle :global(.icon) {
    width: 20px;
    height: 20px;
  }
  .play {
    flex: 1;
    min-width: 0;
    display: grid;
    grid-template-columns: 44px 2fr 1.4fr 1.4fr;
    align-items: center;
    column-gap: 14px;
    padding: 9px 8px 9px 16px;
    text-align: left;
  }
  .numbered .play {
    grid-template-columns: 30px 2fr 1.4fr;
  }
  .n {
    color: var(--ink2);
    font-variant-numeric: tabular-nums;
    display: flex;
    justify-content: center;
  }
  .art {
    width: 44px;
  }
  .title {
    font-size: 16px;
    line-height: 21px;
  }
  .now .title {
    color: var(--accent);
  }
  .artist,
  .album {
    color: var(--ink2);
  }
  .starred {
    color: var(--accent);
  }
  .starred :global(.icon) {
    width: 15px;
    height: 15px;
  }
  .len {
    color: var(--ink2);
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    min-width: 44px;
    text-align: right;
  }
  .more {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    color: var(--ink2);
    flex: none;
  }
  .more :global(.icon) {
    width: 20px;
    height: 20px;
  }
  .more:hover {
    background: var(--hover);
  }
  @media (hover: hover) {
    .more {
      opacity: 0;
    }
    li:hover .more,
    .more:focus-visible {
      opacity: 1;
    }
  }
  @media (max-width: 719px) {
    .play {
      grid-template-columns: 44px 1fr;
      row-gap: 1px;
      padding-left: 12px;
    }
    .numbered .play {
      grid-template-columns: 30px 1fr;
    }
    .art,
    .n {
      grid-row: span 2;
    }
    .title,
    .artist {
      grid-column: 2;
    }
    .artist {
      font-size: 13px;
    }
    .album {
      display: none;
    }
    li::after {
      left: 70px;
    }
    .numbered li::after {
      left: 56px;
    }
  }
  /* The song playing: three bars in the accent, moving while it plays. */
  .bars {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 14px;
  }
  .bars i {
    width: 3px;
    height: 100%;
    border-radius: 1px;
    background: var(--accent);
    transform-origin: bottom;
    animation: bar 900ms ease-in-out infinite alternate;
  }
  .bars i:nth-child(2) {
    animation-delay: -300ms;
  }
  .bars i:nth-child(3) {
    animation-delay: -600ms;
  }
  .bars.still i {
    animation-play-state: paused;
  }
  @keyframes bar {
    from {
      transform: scaleY(0.25);
    }
  }
</style>
