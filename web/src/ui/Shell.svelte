<script lang="ts">
  // The app signed in: the page, scrolling under the chrome. On a phone the chrome is one outlined pill at
  // the bottom, the mini player over the tabs (the tabs alone while nothing plays); on a wide screen an
  // outlined bar along the top and the player's along the bottom; on a wide one the side sheet docks as a third
  // outlined pill beside the page, whose content makes room for it (`--dock`), the two bars centered on what is
  // left, the page's backdrop running on behind it. The full player is a layer over them.
  // Home comes with the shell; the other pages and the full player load once the first page has drawn,
  // so the first paint carries only what it shows.
  import { untrack, type Component } from "svelte";
  import { drawn } from "../state/library.svelte";
  import { itemMenu } from "../state/menus";
  import { player } from "../state/player.svelte";
  import { route } from "../state/route.svelte";
  import { worn } from "../state/theme.svelte";
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
    mix: () => import("../pages/Mix.svelte"),
    genres: () => import("../pages/Genres.svelte"),
    genre: () => import("../pages/Genre.svelte"),
    decades: () => import("../pages/Decades.svelte"),
    decade: () => import("../pages/Decade.svelte"),
    songs: () => import("../pages/Songs.svelte"),
    history: () => import("../pages/History.svelte"),
    stats: () => import("../pages/Stats.svelte"),
    favorites: () => import("../pages/Favorites.svelte"),
    search: () => import("../pages/Search.svelte"),
    settings: () => import("../pages/Settings.svelte"),
    equalizer: () => import("../pages/Equalizer.svelte"),
    support: () => import("../pages/Support.svelte"),
  };
  const nowPlaying = () => import("./NowPlaying.svelte");
  const lyricsInfo = () => import("./LyricsInfo.svelte");
  // Fetched while the browser is idle after the first page, so nothing waits for them later.
  void drawn.then(() => {
    const all = () => [...Object.values(pages), nowPlaying, lyricsInfo].forEach((f) => void f());
    if ("requestIdleCallback" in window) requestIdleCallback(all, { timeout: 3000 });
    else setTimeout(all, 300);
  });

  $effect(() => {
    void route.visit;
    untrack(() => ui.closeAll());
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
      case "decade":
        return { start: Number(a) };
      default:
        return { id: a };
    }
  });
</script>

<!-- Every item's right click opens its menu, wherever it is drawn. -->
<svelte:window oncontextmenucapture={itemMenu} />

<div class="shell tinted recolors" class:docked={ui.docked} class:easing={ui.easing} style={ui.tint?.vars ?? worn.vars} style:--bar-h={ui.wide ? null : playing ? "132px" : "76px"}>
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
  /* The season's colors, or those a page about one cover lends it (`ui.tint`): the page and the chrome
     cross-fade to them (`recolors`). */
  .shell {
    --dock: 0px;
    --pill-w: min(1040px, calc(100vw - var(--dock) - 32px));
    position: fixed;
    inset: 0;
    background: var(--page);
    color: var(--ink);
  }
  /* The desktop's gutter: 3.2 % of the page between 32 and 40 px, wider past a page 1520 px wide so what
     it holds is 1440 px at most. */
  @media (min-width: 720px) {
    .shell {
      --gutter: max(clamp(32px, (100vw - var(--dock)) * 0.032, 40px), (100vw - var(--dock) - 1440px) / 2);
    }
  }
  /* The docked side sheet's room with its inset from the window's edge (the bars'). */
  .shell.docked {
    --dock: 336px;
  }
  /* The page scrolls under the chrome, which floats over it; its content ends where a docked side sheet
     begins, its backdrop (a cover's wash) runs on behind it. */
  main {
    container-type: inline-size;
    position: fixed;
    inset: 0;
    padding-right: var(--dock);
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  /* A page's backdrop (a cover's wash, an artist's picture), placed in its page: from the window's left
     edge, across the page's content (`cqw`, whatever column the page stands in) and on behind a docked side
     sheet. */
  main :global(.backdrop) {
    left: calc((100% - 100cqw) / 2);
    width: calc(100cqw + var(--dock));
  }
  /* The sheet opened or shut by a press: the page and its bars ease to their place with the sheet's glide
     (svelte's cubicOut). */
  .shell.easing main,
  .shell.easing :global(.top),
  .shell.easing :global(.bar) {
    transition:
      padding-right 240ms cubic-bezier(0.33, 1, 0.68, 1),
      left 240ms cubic-bezier(0.33, 1, 0.68, 1),
      width 240ms cubic-bezier(0.33, 1, 0.68, 1);
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
    left: calc((100% - var(--dock)) / 2);
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
