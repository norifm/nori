<script lang="ts">
  // Songs as rows: a click plays them from there, as the page's list (`kind`, `id`); each has its menu
  // (`extra`: the page's own entries). `numbered` shows track numbers instead of covers (an album);
  // `shown` shows only the first few, with Show all under them; `moved`, each row's handle drags it to
  // another place (a playlist's).
  import { player } from "../state/player.svelte";
  import { songMenu } from "../state/menus";
  import type { Item } from "../state/ui.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import { sortable } from "./sortable";

  let { songs, kind = "", id = "", numbered = false, shown, moved, extra = () => [] }: { songs: Song[]; kind?: string; id?: string; numbered?: boolean; shown?: number; moved?: (from: number, to: number) => unknown; extra?: (s: Song, i: number) => Item[] } = $props();
  const playingId = $derived(player.song?.id);
  let all = $state(false);
  const rows = $derived(all || shown === undefined ? songs : songs.slice(0, shown));
</script>

<ol class="songs" class:numbered use:sortable={{ moved: (from, to) => moved?.(from, to) }}>
  {#each rows as s, i (`${i}:${s.id}`)}
    <li class:now={s.id === playingId} data-row={moved ? "" : undefined} data-at={i}>
      <button class="play" onclick={() => void player.play(songs, i, false, kind, id)}>
        {#if numbered}
          <span class="n">{#if s.id === playingId}<Icon name={player.playing ? "play-solid" : "pause-solid"} />{:else}{s.track || i + 1}{/if}</span>
        {:else}
          <span class="art"><Cover src={s.cover} /></span>
        {/if}
        <span class="title clip">{s.title}</span>
        <span class="artist clip">{s.artist}</span>
        {#if !numbered}<span class="album clip">{s.album}</span>{/if}
      </button>
      {#if s.starred}<span class="starred" aria-label={W.favorite}><Icon name="sealed" /></span>{/if}
      <span class="len">{W.time(s.duration * 1000)}</span>
      <button class="round more" aria-label={W.more} title={W.more} onclick={() => void songMenu(s, extra(s, i))}><Icon name="more-horiz" /></button>
      {#if moved}<span class="handle" data-handle aria-label={W.reorder} title={W.reorder}><Icon name="menu" /></span>{/if}
    </li>
  {/each}
</ol>
{#if rows.length < songs.length}<button class="pill all" onclick={() => (all = true)}>{W.showAll}</button>{/if}

<style>
  .all {
    margin-top: 8px;
  }
  .songs {
    list-style: none;
    margin: 0;
    padding: 0;
    background: var(--surface);
    border: 1px solid var(--hair);
    border-radius: 18px;
    overflow: hidden;
  }
  li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-right: 8px;
    border-bottom: 1px solid var(--hair);
  }
  li:last-child {
    border-bottom: 0;
  }
  li:hover {
    background: var(--hover);
  }
  li:global(.held) {
    position: relative;
    z-index: 1;
    background: var(--surface);
    box-shadow: 0 0 0 1px var(--hair);
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
    display: grid;
    grid-template-columns: 44px 2fr 1.4fr 1.4fr;
    align-items: center;
    column-gap: 14px;
    padding: 8px 8px 8px 16px;
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
  .n :global(.icon) {
    color: var(--accent);
    width: 16px;
    height: 16px;
  }
  .art {
    width: 44px;
  }
  .art :global(.cover) {
    border-radius: 7px;
  }
  .now .title {
    color: var(--accent);
    font-weight: 600;
  }
  .artist,
  .album {
    color: var(--ink2);
  }
  .starred {
    color: var(--accent);
  }
  .starred :global(.icon) {
    width: 14px;
    height: 14px;
  }
  .len {
    color: var(--ink2);
    font-variant-numeric: tabular-nums;
    min-width: 44px;
    text-align: right;
  }
  .more {
    color: var(--ink2);
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
    .art {
      grid-row: span 2;
    }
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
</style>
