<script lang="ts">
  // The top bar's search: a field that widens when focused and, as you type, a preview under it of the
  // best few artists, albums, playlists and songs (the core's `search_preview` of its ranked answer), then
  // a way to every result. Arrows walk the preview, Enter opens the line walked to or the search page.
  // Read a moment after the typing stops; the last answer stays until the next comes. Not on the search
  // page, which shows the answer itself.
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { ask, read } from "../rpc";
  import { go, route, searchFor } from "../state/route.svelte";
  import { player } from "../state/player.svelte";
  import type { Album, Artist, Playlist, Song } from "../state/types";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  type Kind = "artists" | "albums" | "playlists" | "songs";
  type Line = { line: "head"; kind: Kind } | { line: "artist" | "album" | "playlist" | "song"; at: number } | { line: "all" };
  type Found = { artists: Artist[]; albums: Album[]; playlists: Playlist[]; songs: Song[]; preview: Line[] };
  let query = $state("");
  // The search page's query, however it was reached (typed, a link, Back).
  $effect(() => {
    if (route.top === "search") query = route.arg;
  });
  let found = $state.raw<Found | null>(null);
  let open = $state(false);
  let focused = $state(false);
  /** The line the arrows are on, by its place in the preview; -1 for none. */
  let at = $state(-1);
  let box = $state<HTMLDivElement>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let asked = 0;
  const still = () => document.documentElement.classList.contains("still");

  function typed() {
    clearTimeout(timer);
    const q = query.trim();
    // A read still out when the box is cleared is no longer wanted.
    const mine = ++asked;
    if (!q) {
      found = null;
      open = false;
      return;
    }
    open = true;
    let answered = false;
    // The offline index at once, as the desktop's search; then the server's better answer.
    void read<Found | null>({ kind: "searchIndex", query: q })
      .then((p) => {
        if (mine !== asked || answered || !p) return;
        found = p;
        at = -1;
      })
      .catch(() => {});
    void ask<number>("search_delay", q).then((ms) => {
      if (mine !== asked) return;
      timer = setTimeout(async () => {
        const p = await read<Found>({ kind: "search", query: q }).catch(() => null);
        if (mine !== asked || !p) return;
        answered = true;
        // A server that finds nothing for a letter or two takes nothing from what the index found.
        if (!p.preview?.length && found?.preview?.length) return;
        found = p;
        at = -1;
      }, ms);
    });
  }

  const shown = $derived(open && !!found && route.top !== "search");
  const lines = $derived(found?.preview ?? []);
  /** The lines the arrows stop on: all but the groups' heads. */
  const stops = $derived(lines.flatMap((l, i) => (l.line === "head" ? [] : [i])));

  const close = () => {
    open = false;
    at = -1;
  };
  function choose(l: Line) {
    const f = found;
    close();
    if (!f) return;
    switch (l.line) {
      case "artist":
        return go("artist", f.artists[l.at].id);
      case "album":
        return go("album", f.albums[l.at].id);
      case "playlist":
        return go("playlist", f.playlists[l.at].id);
      case "song":
        return void player.play(f.songs.slice(l.at, l.at + 1), 0);
      case "all":
        return searchFor(query);
    }
  }

  function keys(e: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (e.key === "Escape") {
      e.preventDefault();
      if (shown) close();
      else e.currentTarget.blur();
    } else if ((e.key === "ArrowDown" || e.key === "ArrowUp") && shown && stops.length) {
      e.preventDefault();
      const k = stops.indexOf(at);
      const next = e.key === "ArrowDown" ? (k + 1) % stops.length : k <= 0 ? stops.length - 1 : k - 1;
      at = stops[next];
      box?.querySelector(`[data-line="${at}"]`)?.scrollIntoView({ block: "nearest" });
    }
  }
  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (shown && at >= 0) return choose(lines[at]);
    close();
    if (query.trim()) searchFor(query);
  }

  const heads: Record<Kind, string> = { artists: W.artists, albums: W.albums, playlists: W.playlists, songs: W.songs };
</script>

<svelte:window onpointerdown={(e) => open && !box?.contains(e.target as Node) && close()} />

<div class="box" bind:this={box}>
  <form class="search" class:on={route.tab === "search" || focused} role="search" onsubmit={submit}>
    <Icon name="search" />
    <input
      type="search"
      placeholder={focused ? W.searchHint : W.search}
      aria-label={W.search}
      aria-expanded={shown}
      aria-controls="search-preview"
      aria-activedescendant={shown && at >= 0 ? `search-line-${at}` : undefined}
      role="combobox"
      autocomplete="off"
      bind:value={query}
      oninput={typed}
      onfocus={() => ((focused = true), query.trim() && (open = true))}
      onblur={() => (focused = false)}
      onkeydown={keys}
    />
  </form>
  {#if shown && found}
    <div class="popover results" id="search-preview" role="listbox" aria-label={W.search} transition:fly={{ y: -6, duration: still() ? 0 : 160, easing: cubicOut }}>
      {#if !lines.length}<p class="empty">{W.noResults}</p>{/if}
      {#each lines as l, i (i)}
        {#if l.line === "head"}
          <div class="cap" class:first={i === 0}>{heads[l.kind]}</div>
        {:else}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div class="res" class:all={l.line === "all"} class:lit={i === at} id="search-line-{i}" data-line={i} role="option" tabindex="-1" aria-selected={i === at} onpointerenter={() => (at = i)} onclick={() => choose(l)}>
            {#if l.line === "artist"}
              {@const a = found.artists[l.at]}
              <span class="art"><Cover src={a.cover} round fallback="user" /></span><span class="words"><span class="clip">{a.name}</span></span>
            {:else if l.line === "album"}
              {@const a = found.albums[l.at]}
              <span class="art"><Cover src={a.cover} radius="var(--inner-art)" fallback="compact-disc" /></span><span class="words"><span class="clip">{a.name}</span><span class="sub clip">{a.artist}</span></span>
            {:else if l.line === "playlist"}
              {@const p = found.playlists[l.at]}
              <span class="art"><Cover src={p.cover} radius="var(--inner-art)" fallback="playlist" /></span><span class="words"><span class="clip">{p.name}</span><span class="sub clip">{W.songCount(p.songCount)}</span></span>
            {:else if l.line === "song"}
              {@const s = found.songs[l.at]}
              <span class="art"><Cover src={s.cover} radius="var(--inner-art)" /></span><span class="words"><span class="clip">{s.title}</span><span class="sub clip">{s.artist}</span></span>
              <Icon name="play-solid" />
            {:else}
              <span class="words"><span class="clip">{W.moreResults}</span></span><Icon name="nav-arrow-right" />
            {/if}
          </div>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .box {
    position: relative;
  }
  /* As the desktop's: a capsule in the top pill that widens while it is typed in. */
  .search {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 180px;
    height: 36px;
    padding: 0 12px 0 10px;
    border-radius: 18px;
    background: var(--surface);
    border: 1px solid var(--hair);
    transition: width 200ms ease-out;
    color: var(--ink2);
  }
  .search:hover {
    background: color-mix(in srgb, var(--ink) 4%, var(--surface));
  }
  .search.on {
    color: var(--accent);
  }
  .search:focus-within {
    width: 360px;
    border-color: var(--accent);
  }
  .search :global(.icon) {
    width: 18px;
    height: 18px;
    flex: none;
  }
  @media (max-width: 1023px) {
    .search {
      width: 36px;
      padding: 0 8px;
    }
    .search:focus-within {
      width: 300px;
      padding: 0 12px 0 10px;
    }
  }
  input {
    all: unset;
    flex: 1;
    min-width: 0;
    color: var(--ink);
    font-size: 13px;
  }
  input::placeholder {
    color: var(--ink2);
  }
  input:focus::placeholder {
    color: color-mix(in srgb, var(--ink) 35%, transparent);
  }
  input::-webkit-search-cancel-button {
    display: none;
  }
  /* A popover, as the menus: hanging from the field, as wide as it, as the desktop's. Its rows' corners are
     the tile's less the inset; a cover's are the row's less the 6 px it sits in. */
  .results {
    --inner-art: calc(var(--inner) - 6px);
    position: absolute;
    top: 46px;
    left: 0;
    right: 0;
    max-height: calc(100vh - 160px);
    overflow-y: auto;
    z-index: 5;
  }
  .cap {
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--ink2);
    padding: 12px 10px 4px;
    border-top: 1px solid var(--line);
    margin-top: 6px;
  }
  .cap.first {
    border-top: 0;
    margin-top: 0;
    padding-top: 6px;
  }
  .res {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 48px;
    padding: 6px;
    border-radius: var(--inner);
    cursor: pointer;
  }
  .res.lit {
    background: var(--hover);
  }
  .art {
    width: 36px;
    flex: none;
  }
  .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    font-size: 14px;
    line-height: 18px;
  }
  .sub {
    font-size: 12.5px;
    color: var(--ink2);
  }
  .res > :global(.icon) {
    width: 16px;
    height: 16px;
    flex: none;
    margin-right: 6px;
    color: var(--ink2);
    opacity: 0;
  }
  .res.lit > :global(.icon),
  .res.all > :global(.icon) {
    opacity: 1;
  }
  .res.all {
    min-height: 40px;
    margin-top: 6px;
    padding-left: 10px;
    border-top: 1px solid var(--line);
    border-radius: 0 0 var(--inner) var(--inner);
    color: var(--ink2);
  }
  .res.all.lit {
    color: var(--ink);
  }
  .empty {
    margin: 0;
    padding: 12px 10px;
    color: var(--ink2);
    font-size: 14px;
  }
</style>
