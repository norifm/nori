<script lang="ts">
  // The queue in play order: what played (dimmed), the song heard, what plays next. A click plays a
  // song; its menu moves or removes it, and a song still to come is dragged anywhere on its row to
  // another place (only while play order is list order). As the desktop's, rows glide to where a move or
  // the next song puts them, and come and go folding.
  import { player } from "../state/player.svelte";
  import { songMenu } from "../state/menus";
  import { beside, pointer } from "../state/ui.svelte";
  import { W } from "../words";
  import type { QueueSong, Song } from "../state/types";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import JamQueue from "./JamQueue.svelte";
  import { sortable } from "./sortable";
  import { flip } from "svelte/animate";
  import { cubicOut } from "svelte/easing";
  import { slide } from "svelte/transition";

  const n = $derived(player.now);
  const rows = $derived(n?.queue ?? []);
  const current = $derived(n?.current ?? -1);
  let list = $state<HTMLOListElement>();
  /** A row's glide, ms, on one curve for both ways rows move at once ([cubicOut] in CSS), so they keep step. */
  const GLIDE_MS = 240;
  const CUBIC_OUT = "cubic-bezier(0.33, 1, 0.68, 1)";
  const still = () => document.documentElement.classList.contains("still");
  /** The rows and their captions, each kept by what it is (a song by its id and which copy of it), so a
   * song moved is the same row moved. */
  const items = $derived.by(() => {
    const copies = new Map<string, number>();
    const out: { key: string; head?: string; q?: QueueSong; at: number }[] = [];
    rows.forEach((q, at) => {
      if (at === 0 && current > 0) out.push({ key: "history", head: W.history, at });
      if (at === current + 1) out.push({ key: "next", head: W.playingNext, at });
      const id = q.id ?? q.title ?? "";
      const copy = copies.get(id) ?? 0;
      copies.set(id, copy + 1);
      out.push({ key: `${id}#${copy}`, q, at });
    });
    return out;
  });
  /** Rows glide to their new places, except while a dropped row lands where it was drawn already. */
  const moved = (node: Element, places: { from: DOMRect; to: DOMRect }) =>
    flip(node, places, { duration: still() || node.closest(".landing") ? 0 : GLIDE_MS, easing: cubicOut });
  // The song heard is the list's first row, what played above it: when the list opens, and when the next
  // song takes over while the list rests on the one before. Scrolled somewhere else, it stays where it
  // was put; a song clicked in it is followed wherever the list was.
  let pinned = true;
  /** Opened at the song heard at once; after that it glides to the next. */
  let placed = false;
  const playing = () => list?.querySelector<HTMLElement>(".current");
  $effect(() => {
    void current;
    const el = playing();
    if (!list || !el || !pinned) return;
    // Where it is laid out, not where a glide draws it just now. Followed to the next song, the rows in
    // sight glide up from where they were drawn rather than the list jumping.
    const by = el.offsetTop - list.scrollTop;
    list.scrollTop = el.offsetTop;
    if (placed && by && !still()) glide(list, by);
    placed = true;
  });
  /** The rows in sight drawn `by` px lower than they are, gliding home; the captions fade back in. */
  function glide(list: HTMLElement, by: number) {
    const top = list.scrollTop - Math.abs(by);
    const bottom = list.scrollTop + list.clientHeight + Math.abs(by);
    for (const row of list.children as HTMLCollectionOf<HTMLElement>) {
      if (row.offsetTop + row.offsetHeight < top || row.offsetTop > bottom) continue;
      row.animate([{ transform: `translateY(${by}px)` }, { transform: "none" }], { duration: GLIDE_MS, easing: CUBIC_OUT, composite: "add" });
      // A caption the song passes on its way up comes back where it now is rather than under it.
      if (row.classList.contains("head")) row.animate([{ opacity: 0 }, { opacity: 0, offset: 0.85 }, { opacity: 1 }], { duration: GLIDE_MS + 120 });
    }
  }
  /** Where a scroll left the list decides whether it follows the next song. */
  function rested() {
    const el = playing();
    if (!list || !el) return;
    pinned = Math.abs(el.offsetTop - list.scrollTop) < el.offsetHeight / 2;
  }
  const jump = (q: QueueSong) => {
    pinned = true;
    void player.control("jump", q.index);
  };
  const move = (from: number, to: number) => player.control("move_song", from, to);
  const extra = (q: QueueSong, at: number) => [
    ...(n?.reorderable && at > 0 ? [{ label: W.moveUp, icon: "nav-arrow-up", run: () => move(q.index, rows[at - 1].index) }] : []),
    ...(n?.reorderable && at < rows.length - 1 ? [{ label: W.moveDown, icon: "nav-arrow-down", run: () => move(q.index, rows[at + 1].index) }] : []),
    ...(at !== current ? [{ label: W.remove, icon: "xmark", run: () => player.control("remove", q.index) }] : []),
  ];
</script>

<JamQueue />
<ol class="queue" id="queue" bind:this={list} onscrollend={rested} use:sortable={{ moved: (from, to) => move(rows[from].index, rows[to].index), whole: true }}>
  {#each items as item (item.key)}
    {@const q = item.q}
    {@const at = item.at}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <li
      class={q ? "row" : "head"}
      animate:moved
      transition:slide={{ duration: still() ? 0 : 180 }}
      oncontextmenu={(e) => q?.id && (e.preventDefault(), void songMenu(q as Song, pointer(e), { play: () => jump(q), extra: extra(q, at) }))}
      class:current={q && at === current}
      class:past={q && at < current}
      data-row={q && n?.reorderable && at > current ? "" : undefined}
      data-at={q ? at : undefined}
    >
      {#if q}
        <button class="play" onclick={() => jump(q)}>
          <span class="art"><Cover src={q.cover} /></span>
          <span class="text">
            <span class="title clip">{q.title ?? q.id ?? ""}</span>
            <span class="sub clip">{q.by ? W.jamAddedBy(q.by) : (q.artist ?? "")}</span>
          </span>
          {#if at === current}<Icon name={player.playing ? "play-solid" : "pause-solid"} />{/if}
        </button>
        {#if q.id}
          <button class="more" data-still aria-label={W.more} title={W.more} onclick={(e) => void songMenu(q as Song, beside(e), { play: () => jump(q), extra: extra(q, at) })}><Icon name="more-horiz" /></button>
        {/if}
      {:else}{item.head}{/if}
    </li>
  {:else}
    <li class="none">{W.nothingPlaying}</li>
  {/each}
</ol>

<style>
  .queue {
    position: relative;
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }
  /* Room after the last song, so any song, the last too, can be the list's first row. */
  .queue::after {
    content: "";
    display: block;
    height: calc(100% - 52px);
  }
  .head {
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--ink2);
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
    box-shadow: 0 0 0 var(--hairline) var(--hair);
  }
  /* A row to come is taken anywhere: no words picked or callout on a long press. */
  .row[data-row] {
    user-select: none;
    -webkit-user-select: none;
    -webkit-touch-callout: none;
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
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title {
    font-size: 15px;
  }
  .more {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    color: var(--ink2);
    opacity: 0;
  }
  .more :global(.icon) {
    width: 20px;
    height: 20px;
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
