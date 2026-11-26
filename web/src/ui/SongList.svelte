<script lang="ts">
  // Songs as rows on the page's paper, under their columns' heads as the desktop's: a click plays them from
  // there, as the page's list (`kind`, `id`); each has its menu (`extra`: the page's own entries).
  // `numbered` shows track numbers instead of covers (an album's); `rows` the positions shown (a disc's);
  // `shown` only the first few, with Show all under them; `moved`, each row's handle drags it to another
  // place (a playlist's); `one`, a click plays that song alone (a search's results).
  import { player } from "../state/player.svelte";
  import { songMenu } from "../state/menus";
  import { beside, pointer, type Item } from "../state/ui.svelte";
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
  }: { songs: Song[]; kind?: string; id?: string; numbered?: boolean; rows?: number[]; shown?: number; moved?: (from: number, to: number) => unknown; one?: boolean; extra?: (s: Song, i: number) => Item[] | Promise<Item[]> } = $props();
  const playingId = $derived(player.song?.id);
  let all = $state(false);
  const at = $derived(rows ?? songs.map((_, i) => i));
  const drawn = $derived(all || shown === undefined ? at : at.slice(0, shown));
  /** What a click on row `i` does, and its menu's Play. */
  const play = (i: number) => (one ? player.play([songs[i]], 0, false, kind, id) : player.play(songs, i, false, kind, id));
</script>

{#if !numbered}
  <div class="heads" aria-hidden="true">
    <span class="cols"><span></span><span>{W.songHeads.song}</span><span>{W.songHeads.artist}</span><span>{W.songHeads.album}</span></span>
    <span class="seal"></span><span class="len">{W.songHeads.time}</span><span class="room"></span>{#if moved}<span class="room grip"></span>{/if}
  </div>
{/if}
<ol class="songs" class:numbered use:sortable={{ moved: (from, to) => moved?.(from, to) }}>
  {#each drawn as i (`${i}:${songs[i].id}`)}
    {@const s = songs[i]}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <li oncontextmenu={(e) => (e.preventDefault(), void songMenu(s, pointer(e), { play: () => play(i), extra: extra(s, i) }))} class:now={s.id === playingId} data-row={moved ? "" : undefined} data-at={i}>
      <button class="play" onclick={() => void play(i)}>
        {#if numbered}
          <span class="n">{#if s.id === playingId}<span class="bars" class:still={!player.playing}><i></i><i></i><i></i></span>{:else}{s.track || i + 1}{/if}</span>
        {:else}
          <span class="art"><Cover src={s.cover} /></span>
        {/if}
        <span class="title clip">{s.title}</span>
        <span class="artist clip">{s.artist}</span>
        {#if !numbered}<span class="album clip">{s.album}</span>{/if}
      </button>
      <span class="seal">{#if s.starred}<span class="starred" aria-label={W.favorite}><Icon name="sealed" /></span>{/if}</span>
      <span class="len">{W.time(s.duration * 1000)}</span>
      <button class="more" aria-label={W.more} title={W.more} onclick={(e) => void songMenu(s, beside(e), { play: () => play(i), extra: extra(s, i) })}><Icon name="more-horiz" /></button>
      {#if moved}<span class="handle" data-handle aria-label={W.reorder} title={W.reorder}><Icon name="menu" /></span>{/if}
    </li>
  {/each}
</ol>
{#if drawn.length < at.length}<button class="text-action all" onclick={() => (all = true)}>{W.showAll}</button>{/if}

<style>
  .all {
    margin-top: 8px;
  }
  /* The rows' words start at the page's edge; their hover reaches 8 px past it. */
  .songs {
    list-style: none;
    margin: 0 -8px;
    padding: 0;
  }
  li,
  .heads {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    padding-right: 8px;
  }
  li {
    border-radius: 10px;
  }
  .heads {
    margin: 0 -8px 2px;
    height: 28px;
    color: var(--ink2);
    font-size: 12px;
    font-weight: 600;
  }
  .heads .cols {
    flex: 1;
    min-width: 0;
    display: grid;
    grid-template-columns: 44px 2fr 1.4fr 1.4fr;
    column-gap: 14px;
    padding: 0 8px;
  }
  .heads .room {
    width: 40px;
    flex: none;
  }
  .heads .grip {
    width: 32px;
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
    padding: 9px 8px;
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
  /* A column of its own, so the times stand under their head whether a row is sealed or not. */
  .seal {
    display: flex;
    width: 20px;
    flex: none;
  }
  .starred {
    color: var(--accent);
  }
  /* The seal the size of the row's other glyphs, so its line is theirs. */
  .starred :global(.icon) {
    width: 20px;
    height: 20px;
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
    .heads {
      display: none;
    }
    .seal:empty {
      display: none;
    }
    .play {
      grid-template-columns: 44px 1fr;
      row-gap: 1px;
      padding-left: 8px;
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
