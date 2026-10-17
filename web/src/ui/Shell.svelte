<script lang="ts">
  // The app signed in: on a wide screen the sidebar, the page, the panel beside it and the player bar
  // under them, as the desktop app and Apple Music have them; on a phone the page, the mini player and
  // the tab bar, the full player a sheet over them.
  import { library } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { route } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import Sidebar from "./Sidebar.svelte";
  import TabBar from "./TabBar.svelte";
  import PlayerBar from "./PlayerBar.svelte";
  import MiniPlayer from "./MiniPlayer.svelte";
  import NowPlaying from "./NowPlaying.svelte";
  import Panel from "./Panel.svelte";
  import Sheet from "./Sheet.svelte";
  import Prompt from "./Prompt.svelte";
  import LyricsInfo from "./LyricsInfo.svelte";
  import Home from "../pages/Home.svelte";
  import Library from "../pages/Library.svelte";
  import Albums from "../pages/Albums.svelte";
  import Album from "../pages/Album.svelte";
  import Artists from "../pages/Artists.svelte";
  import Artist from "../pages/Artist.svelte";
  import Playlists from "../pages/Playlists.svelte";
  import Playlist from "../pages/Playlist.svelte";
  import Genres from "../pages/Genres.svelte";
  import Genre from "../pages/Genre.svelte";
  import Songs from "../pages/Songs.svelte";
  import Favorites from "../pages/Favorites.svelte";
  import Search from "../pages/Search.svelte";
  import Settings from "../pages/Settings.svelte";

  void library.readPlaylists().catch(() => {});
  const playing = $derived(!!player.song);
</script>

<div class="shell" class:wide={ui.wide} class:playing class:paneled={ui.wide && !!ui.panel}>
  {#if ui.wide}<Sidebar />{/if}
  <main id="main">
    {#if route.top === "library"}<Library />
    {:else if route.top === "albums"}<Albums sort={route.arg || "alphabeticalByName"} />
    {:else if route.top === "album"}<Album id={route.arg} />
    {:else if route.top === "artists"}<Artists />
    {:else if route.top === "artist"}<Artist id={route.arg} />
    {:else if route.top === "playlists"}<Playlists />
    {:else if route.top === "playlist"}<Playlist id={route.arg} />
    {:else if route.top === "genres"}<Genres />
    {:else if route.top === "genre"}<Genre name={route.arg} />
    {:else if route.top === "songs"}<Songs />
    {:else if route.top === "favorites"}<Favorites />
    {:else if route.top === "search"}<Search query={route.arg} />
    {:else if route.top === "settings"}<Settings />
    {:else}<Home />{/if}
  </main>
  {#if ui.wide && ui.panel}<Panel />{/if}
  {#if ui.wide}
    <PlayerBar />
  {:else}
    {#if playing}<MiniPlayer />{/if}
    <TabBar />
  {/if}
</div>
{#if ui.full && playing}<NowPlaying />{/if}
{#if ui.lyricsInfo}<LyricsInfo />{/if}
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
