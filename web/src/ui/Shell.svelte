<script lang="ts">
  // The app signed in: on a wide screen the sidebar, the page, the panel beside it and the player bar
  // under them, as the desktop app and Apple Music have them; on a phone the page, the mini player and
  // the tab bar, the full player a sheet over them. Home comes with the shell; the other pages, the full
  // player and the panel load once the first page has drawn, so the first paint carries only what it
  // shows.
  import type { Component } from "svelte";
  import { drawn, library } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { route } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import Sidebar from "./Sidebar.svelte";
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
  const panel = () => import("./Panel.svelte");
  const lyricsInfo = () => import("./LyricsInfo.svelte");
  // Fetched while the browser is idle after the first page, so nothing waits for them later.
  void drawn.then(() => {
    const all = () => [...Object.values(pages), nowPlaying, panel, lyricsInfo].forEach((f) => void f());
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

<div class="shell" class:wide={ui.wide} class:playing class:paneled={ui.wide && !!ui.panel}>
  {#if ui.wide}<Sidebar />{/if}
  <main id="main">
    {#if pages[route.top]}
      {#await pages[route.top]() then m}<m.default {...props} />{/await}
    {:else}<Home />{/if}
  </main>
  {#if ui.wide && ui.panel}{#await panel() then m}<m.default />{/await}{/if}
  {#if ui.wide}
    <PlayerBar />
  {:else}
    {#if playing}<MiniPlayer />{/if}
    <TabBar />
  {/if}
</div>
{#if ui.full && playing}{#await nowPlaying() then m}<m.default />{/await}{/if}
{#if ui.lyricsInfo}{#await lyricsInfo() then m}<m.default />{/await}{/if}
{#if ui.sheet}<Sheet />{/if}
{#if ui.prompt}<Prompt />{/if}
{#if ui.toast}<div class="toast" role="status">{ui.toast}</div>{/if}

<style>
  .shell {
    min-height: 100vh;
    padding-bottom: calc(var(--tabs-h) + env(safe-area-inset-bottom));
  }
  .shell.playing {
    padding-bottom: calc(var(--tabs-h) + var(--mini-h) + 8px + env(safe-area-inset-bottom));
  }
  /* Wide: the sidebar and the panel stay put; the page scrolls between them, above the bar. */
  .shell.wide {
    display: grid;
    grid-template-columns: var(--side-w) minmax(0, 1fr);
    height: 100vh;
    padding: 0 0 var(--bar-h);
  }
  .shell.wide.paneled {
    grid-template-columns: var(--side-w) minmax(0, 1fr) var(--panel-w);
  }
  .wide main {
    overflow-y: auto;
    min-height: 0;
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: calc(var(--tabs-h) + var(--mini-h) + 24px);
    transform: translateX(-50%);
    background: var(--raised);
    color: var(--text);
    box-shadow: var(--shadow);
    padding: 10px 18px;
    border-radius: 12px;
    font-size: 14px;
    z-index: 60;
    max-width: calc(100vw - 32px);
  }
  .wide ~ .toast {
    bottom: calc(var(--bar-h) + 20px);
  }
</style>
