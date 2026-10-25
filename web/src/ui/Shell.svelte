<script lang="ts">
  // The app signed in: the page, scrolling under the chrome. On a phone the chrome is one outlined pill at
  // the bottom, the mini player over the tabs (the tabs alone while nothing plays); on a wide screen an
  // outlined bar along the top and the player's along the bottom. The full player is a layer over them.
  // Home comes with the shell; the other pages and the full player load once the first page has drawn,
  // so the first paint carries only what it shows.
  import type { Component } from "svelte";
  import { drawn } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { route } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import TopPill from "./TopPill.svelte";
  import TabBar from "./TabBar.svelte";
  import PlayerBar from "./PlayerBar.svelte";
  import MiniPlayer from "./MiniPlayer.svelte";
  import Sheet from "./Sheet.svelte";
  import SideSheet from "./SideSheet.svelte";
  import Popover from "./Popover.svelte";
  import Prompt from "./Prompt.svelte";
  import Confirm from "./Confirm.svelte";
  import JoinJam from "./JoinJam.svelte";
  import PlaylistPicker from "./PlaylistPicker.svelte";
  import Home from "../pages/Home.svelte";

  type Loaded = Promise<{ default: Component<any> }>;
  const pages: Record<string, () => Loaded> = {
    row: () => import("../pages/Row.svelte"),
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
    equalizer: () => import("../pages/Equalizer.svelte"),
  };
  const nowPlaying = () => import("./NowPlaying.svelte");
  const lyricsInfo = () => import("./LyricsInfo.svelte");
  // Fetched while the browser is idle after the first page, so nothing waits for them later.
  void drawn.then(() => {
    const all = () => [...Object.values(pages), nowPlaying, lyricsInfo].forEach((f) => void f());
    if ("requestIdleCallback" in window) requestIdleCallback(all, { timeout: 3000 });
    else setTimeout(all, 300);
  });

  const playing = $derived(!!player.song);
  /** The page's props, by its address. */
  const props = $derived.by(() => {
    const a = route.arg;
    switch (route.top) {
      case "albums":
        return { sort: a };
      case "genre":
        return { name: a };
      default:
        return { id: a };
    }
  });
</script>

<div class="shell tinted" style={ui.tint?.vars} style:--bar-h={ui.wide ? null : playing ? "132px" : "76px"}>
  <main id="main">
    {#if pages[route.top]}
      {#await pages[route.top]() then m}<m.default {...props} />{/await}
    {:else}<Home />{/if}
  </main>
  {#if ui.wide}
    <TopPill />
    {#if ui.side && !ui.full}<SideSheet pane={ui.side} />{/if}
    <PlayerBar />
  {:else}
    <div class="dock chrome" class:playing>
      {#if playing}<MiniPlayer /><hr />{/if}
      <TabBar />
    </div>
  {/if}
</div>
{#if ui.full && playing}{#await nowPlaying() then m}<m.default />{/await}{/if}
{#if ui.lyricsInfo}{#await lyricsInfo() then m}<m.default />{/await}{/if}
{#if ui.open}{#if ui.open.head && !ui.wide}<Sheet />{:else}<Popover />{/if}{/if}
{#if ui.prompt}<Prompt />{/if}
{#if ui.confirming}<Confirm />{/if}
{#if ui.joining}<JoinJam close={() => (ui.joining = false)} />{/if}
{#if ui.picking}<PlaylistPicker />{/if}
{#if ui.toast}<div class="toast" role="status">{ui.toast}</div>{/if}

<style>
  /* A page about one cover lends it its colors (`ui.tint`): the page and the chrome cross-fade to them. */
  .shell {
    position: fixed;
    inset: 0;
    background: var(--page);
    color: var(--ink);
    transition:
      --page 420ms,
      --surface 420ms,
      --ink 420ms,
      --ink2 420ms,
      --hair 420ms,
      --outline 420ms,
      --accent 420ms;
  }
  /* The page scrolls under the chrome, which floats over it. */
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
    bottom: max(6px, env(safe-area-inset-bottom));
    z-index: 30;
    border-radius: 25px;
    overflow: hidden;
  }
  /* The mini player's cover is 10 px round and 8 px in: the outline is 18 px round. */
  .dock.playing {
    border-radius: 18px;
  }
  hr {
    margin: 0;
    border: 0;
    border-top: 1px solid color-mix(in srgb, var(--ink) 9%, transparent);
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: calc(var(--bar-h) + 12px);
    transform: translateX(-50%);
    background: var(--surface);
    color: var(--ink);
    border: var(--hairline) solid var(--hair);
    padding: 12px 18px;
    border-radius: 24px;
    font-size: 15px;
    z-index: 60;
    max-width: calc(100vw - 32px);
  }
</style>
