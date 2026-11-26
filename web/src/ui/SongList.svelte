<script lang="ts">
  // Songs as rows on the page's paper, under their columns' heads as the desktop's: a click plays them from
  // there, as the page's list (`kind`, `id`); each has its menu (`extra`: the page's own entries).
  // `numbered` shows track numbers instead of covers (an album's); `rows` the positions shown (a disc's);
  // `shown` only the first few, with Show all under them; `moved`, each row's handle drags it to another
  // place (a playlist's); `one`, a click plays that song alone (a search's results).
  import { player } from "../state/player.svelte";
  import { songMenu } from "../state/menus";
  import { beside, pointer, ui, type Item } from "../state/ui.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import PlayingBars from "./PlayingBars.svelte";
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

{#if ui.wide && !numbered && !one}
  <div class="heads" aria-hidden="true">
    <span class="cols"><span></span><span>{W.songHeads.song}</span><span>{W.songHeads.artist}</span><span>{W.songHeads.album}</span></span>
    <span class="seal"></span><span class="len">{W.songHeads.time}</span><span class="room"></span>{#if moved}<span class="room"></span>{/if}
  </div>
{/if}
<ol class="songs" class:numbered use:sortable={{ moved: (from, to) => moved?.(from, to) }}>
  {#each drawn as i (`${i}:${songs[i].id}`)}
    {@const s = songs[i]}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <li oncontextmenu={(e) => (e.preventDefault(), void songMenu(s, pointer(e), { play: () => play(i), extra: extra(s, i) }))} class:now={s.id === playingId} data-row={moved ? "" : undefined} data-at={i}>
      <button class="play" onclick={() => void play(i)}>
        {#if numbered}
          <span class="n">{#if s.id === playingId}<span class="now-bars"><PlayingBars moving={player.advancing} w={14} h={13} /></span>{:else}{s.track || i + 1}{/if}</span>
        {:else}
          <span class="art"><Cover src={s.cover} radius="var(--row-art)" /></span>
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
  li {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    padding-right: 8px;
    border-radius: 10px;
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
  /* The desktop's list on a wide window: rows 56 px high with covers (44 numbered), small words, the song,
     artist and album sharing the room 4:3:3 under their heads. */
  @media (min-width: 720px) {
    .songs {
      --row-art: 5px;
    }
    li {
      border-radius: 7px;
      gap: 8px;
      padding-right: 12px;
    }
    .play {
      grid-template-columns: 44px 4fr 3fr 3fr;
      column-gap: 12px;
      min-height: 56px;
      padding: 6px 0 6px 8px;
      font-size: 13px;
    }
    /* Over the columns, as the desktop's: 28 px high, small and quiet. */
    .heads {
      display: flex;
      align-items: center;
      gap: 8px;
      height: 28px;
      margin: 0 -8px;
      padding-right: 12px;
      font-size: 12px;
      font-weight: 600;
      color: var(--ink2);
    }
    .cols {
      flex: 1;
      min-width: 0;
      display: grid;
      grid-template-columns: 44px 4fr 3fr 3fr;
      column-gap: 12px;
      padding-left: 8px;
    }
    .room {
      width: 32px;
      flex: none;
    }
    .heads .len {
      font-size: 12px;
    }
    .numbered .play {
      grid-template-columns: 24px 2fr 1fr;
      min-height: 44px;
      padding-block: 0;
    }
    .n {
      justify-content: flex-end;
    }
    .title {
      font-size: 13px;
      line-height: 17px;
    }
    .seal {
      width: 15px;
    }
    .starred :global(.icon) {
      width: 15px;
      height: 15px;
    }
    .more {
      width: 32px;
      height: 32px;
    }
  }
  @media (max-width: 719px) {
    .songs {
      --row-art: var(--r-cover);
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
  /* The song playing: its bars in the accent. */
  .now-bars {
    display: flex;
    color: var(--accent);
  }
</style>
