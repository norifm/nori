<script lang="ts">
  // Songs as rows: a click plays them from there, as the page's list (`kind`, `id`); each has its menu
  // (`extra`: the page's own entries). `numbered` shows track numbers instead of covers (an album's);
  // `rows` the positions shown (a disc's), with their second `lines`; `shown` only the first few, with
  // Show all under them; `moved`, each row's handle drags it to another place (a playlist's).
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
    lines,
    shown,
    moved,
    extra = () => [],
  }: { songs: Song[]; kind?: string; id?: string; numbered?: boolean; rows?: number[]; lines?: string[]; shown?: number; moved?: (from: number, to: number) => unknown; extra?: (s: Song, i: number) => Item[] } = $props();
  const playingId = $derived(player.song?.id);
  let all = $state(false);
  const at = $derived(rows ?? songs.map((_, i) => i));
  const drawn = $derived(all || shown === undefined ? at : at.slice(0, shown));
</script>

<ol class="songs" class:numbered use:sortable={{ moved: (from, to) => moved?.(from, to) }}>
  {#each drawn as i, k (`${i}:${songs[i].id}`)}
    {@const s = songs[i]}
    {@const line = lines ? lines[k] : (s.line ?? s.artist)}
    <li class:now={s.id === playingId} data-row={moved ? "" : undefined} data-at={i}>
      <button class="play" onclick={() => void player.play(songs, i, false, kind, id)}>
        {#if numbered}
          <span class="n">{#if s.id === playingId}<span class="bars" class:still={!player.playing}><i></i><i></i><i></i></span>{:else}{s.track || ""}{/if}</span>
        {:else}
          <span class="art"><Cover src={s.cover} /></span>
        {/if}
        <span class="words">
          <span class="title clip">{s.title}</span>
          {#if line}<span class="line clip">{line}</span>{/if}
        </span>
      </button>
      <span class="mark">{#if s.starred}<Icon name="sealed" />{/if}</span>
      <span class="len">{s.duration ? W.time(s.duration * 1000) : ""}</span>
      <button class="more" aria-label={W.more} title={W.more} onclick={() => void songMenu(s, extra(s, i))}><Icon name="more-horiz" /></button>
      {#if moved}<span class="handle" data-handle aria-label={W.reorder} title={W.reorder}><Icon name="menu" /></span>{/if}
    </li>
  {/each}
</ol>
{#if drawn.length < at.length}<button class="text-action all" onclick={() => (all = true)}>{W.showAll}</button>{/if}

<style>
  .songs {
    list-style: none;
    margin: 0 calc(-1 * var(--gutter));
    padding: 0;
  }
  li {
    position: relative;
    display: flex;
    align-items: center;
    padding-right: 4px;
  }
  /* The hairline starts where the text does. */
  li::after {
    content: "";
    position: absolute;
    left: calc(var(--gutter) + 56px);
    right: 0;
    bottom: 0;
    height: 1px;
    background: var(--line);
  }
  .numbered li::after {
    left: calc(var(--gutter) + 40px);
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
  .play {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    padding: 9px 8px 9px var(--gutter);
    text-align: left;
  }
  .art {
    width: 44px;
    flex: none;
  }
  .n {
    width: 26px;
    flex: none;
    display: flex;
    justify-content: center;
    color: var(--ink2);
    font-variant-numeric: tabular-nums;
  }
  .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding-left: 12px;
  }
  .numbered .words {
    padding-left: 14px;
  }
  .title {
    font-size: 16px;
    line-height: 21px;
  }
  .now .title {
    color: var(--accent);
  }
  .line {
    font-size: 13px;
    line-height: 17px;
    color: var(--ink2);
  }
  .mark {
    width: 15px;
    flex: none;
    color: var(--accent);
  }
  .mark :global(.icon) {
    width: 15px;
    height: 15px;
  }
  .len {
    min-width: 40px;
    text-align: right;
    font-size: 13px;
    color: var(--ink2);
    font-variant-numeric: tabular-nums;
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
  .handle {
    display: grid;
    place-items: center;
    width: 36px;
    height: 40px;
    color: var(--ink2);
    cursor: grab;
    touch-action: none;
  }
  .handle :global(.icon) {
    width: 20px;
    height: 20px;
  }
  .all {
    margin: 4px 0 0 -12px;
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
