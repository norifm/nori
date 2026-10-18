<script lang="ts">
  // The app signed in: the page, scrolling under an outlined bar along the top and the player's along the
  // bottom; on a phone one bar at the bottom holds the mini player over the tabs. The full player is a
  // layer over them. Home comes with the shell; the other pages and the full player load once the first
  // page has drawn, so the first paint carries only what it shows.
  import type { Component } from "svelte";
  import { drawn, library } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { route } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import TopPill from "./TopPill.svelte";
  import TabBar from "./TabBar.svelte";
  import PlayerBar from "./PlayerBar.svelte";
  import MiniPlayer from "./MiniPlayer.svelte";
  import Sheet from "./Sheet.svelte";
  import Prompt from "./Prompt.svelte";
  import Home from "../pages/Home.svelte";

  type Loaded = Promise<{ default: Component<any> }>;
  const pages: Record<string, () => Loaded> = {
    library: () => import("../pages/Library.svelte"),
    albums: () => import("../pages/Albums.svelte"),
    album: () => import("../pages/Album.svelte"),
    artists: () => import("../pages/Artists.svelte"),
    artist: () => import("../pages/Artist.svelte"),
    playlists: () => import("../pages/Playlists.svelte"),
    playlist: () => import("../pages/Playlist.svelte"),
    genres: () => import("../pages/Genres.svelte"),
    genre: () => import("../pages/Genre.svelte"),
    songs: () => import("../pages/Songs.svelte"),
    favorites: () => import("../pages/Favorites.svelte"),
    search: () => import("../pages/Search.svelte"),
    settings: () => import("../pages/Settings.svelte"),
  };
  const nowPlaying = () => import("./NowPlaying.svelte");
  const lyricsInfo = () => import("./LyricsInfo.svelte");
  // Fetched while the browser is idle after the first page, so nothing waits for them later.
  void drawn.then(() => {
    const all = () => [...Object.values(pages), nowPlaying, lyricsInfo].forEach((f) => void f());
    if ("requestIdleCallback" in window) requestIdleCallback(all, { timeout: 3000 });
    else setTimeout(all, 300);
  });

  void library.readPlaylists().catch(() => {});
  const playing = $derived(!!player.song);
  /** The page's props, by its address. */
  const props = $derived.by(() => {
    const a = route.arg;
    switch (route.top) {
      case "albums":
        return { sort: a || "alphabeticalByName" };
      case "genre":
        return { name: a };
      case "search":
        return { query: a };
      default:
        return { id: a };
    }
  });
</script>

<div class="shell" class:wide={ui.wide} style:--bar-h={ui.wide ? null : playing ? "148px" : "92px"}>
  <main id="main" class="fade">
    {#if pages[route.top]}
      {#await pages[route.top]() then m}<m.default {...props} />{/await}
    {:else}<Home />{/if}
  </main>
  {#if ui.wide}
    <TopPill />
    <PlayerBar />
  {:else}
    <div class="dock chrome">
      {#if playing}<MiniPlayer /><hr />{/if}
      <TabBar />
    </div>
  {/if}
</div>
{#if ui.full && playing}{#await nowPlaying() then m}<m.default />{/await}{/if}
{#if ui.lyricsInfo}{#await lyricsInfo() then m}<m.default />{/await}{/if}
{#if ui.sheet}<Sheet />{/if}
{#if ui.prompt}<Prompt />{/if}
{#if ui.toast}<div class="toast" class:wide={ui.wide} role="status">{ui.toast}</div>{/if}

<style>
  /* The page scrolls under the bars; they float over it. */
  main {
    position: fixed;
    inset: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .dock {
    position: fixed;
    left: 10px;
    right: 10px;
    bottom: calc(8px + env(safe-area-inset-bottom));
    z-index: 30;
    border-radius: 26px;
    overflow: hidden;
  }
  hr {
    margin: 0 16px;
    border: 0;
    border-top: 1px solid color-mix(in srgb, var(--chrome) 45%, transparent);
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: calc(var(--bar-h) + 8px);
    transform: translateX(-50%);
    background: var(--surface);
    color: var(--ink);
    border: 1px solid var(--hair);
    padding: 10px 18px;
    border-radius: 16px;
    font-size: 14px;
    z-index: 60;
    max-width: calc(100vw - 32px);
  }
</style>
