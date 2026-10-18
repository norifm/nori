<script lang="ts">
  // Songs as rows: a click plays them from there, as the page's list (`kind`, `id`); each has its menu
  // (`extra`: the page's own entries). `numbered` shows track numbers instead of covers (an album).
  import { player } from "../state/player.svelte";
  import { songMenu } from "../state/menus";
  import type { Item } from "../state/ui.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  let { songs, kind = "", id = "", numbered = false, extra = () => [] }: { songs: Song[]; kind?: string; id?: string; numbered?: boolean; extra?: (s: Song, i: number) => Item[] } = $props();
  const playingId = $derived(player.song?.id);
</script>

<ol class="songs" class:numbered>
  {#each songs as s, i (`${i}:${s.id}`)}
    <li class:now={s.id === playingId}>
      <button class="play" onclick={() => void player.play(songs, i, false, kind, id)}>
        {#if numbered}
          <span class="n">{#if s.id === playingId}<Icon name={player.playing ? "play" : "pause"} />{:else}{s.track || i + 1}{/if}</span>
        {:else}
          <span class="art"><Cover src={s.cover} /></span>
        {/if}
        <span class="text">
          <span class="title clip">{s.title}</span>
          <span class="sub clip">{numbered ? s.artist : `${s.artist} · ${s.album}`}</span>
        </span>
      </button>
      {#if s.starred}<span class="starred" aria-label={W.favorite}><Icon name="heart" /></span>{/if}
      <span class="len">{W.time(s.duration * 1000)}</span>
      <button class="round more" aria-label={W.more} title={W.more} onclick={() => void songMenu(s, extra(s, i))}><Icon name="more" /></button>
    </li>
  {/each}
</ol>

<style>
  .songs {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    align-items: center;
    gap: 8px;
    border-radius: 8px;
    padding-right: 4px;
  }
  li:nth-child(odd) {
    background: var(--hover);
  }
  li:hover {
    background: var(--chosen);
  }
  .play {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 10px;
    text-align: left;
  }
  .numbered .play {
    padding: 10px 10px;
  }
  .n {
    width: 22px;
    text-align: right;
    color: var(--dim);
    font-size: 14px;
    font-variant-numeric: tabular-nums;
    display: flex;
    justify-content: flex-end;
  }
  .n :global(.icon) {
    color: var(--accent);
    width: 16px;
    height: 16px;
  }
  .art {
    width: 40px;
    flex: none;
  }
  .art :global(.cover) {
    border-radius: 4px;
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
  .now .title {
    color: var(--accent);
  }
  .numbered .sub {
    display: none;
  }
  .starred {
    color: var(--accent);
  }
  .starred :global(.icon) {
    width: 14px;
    height: 14px;
  }
  .len {
    color: var(--dim);
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    min-width: 40px;
    text-align: right;
  }
  .more {
    color: var(--accent);
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
</style>
