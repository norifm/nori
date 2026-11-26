<script lang="ts" module>
  import { href } from "../state/route.svelte";

  /** A library section's page and glyph, by the core's `LibrarySection`. */
  export const sectionPage: Record<string, [string, string]> = {
    Albums: [href("albums"), "compact-disc"],
    Artists: [href("artists"), "user"],
    Songs: [href("songs"), "music-note"],
    Playlists: [href("playlists"), "playlist"],
    Genres: [href("genres"), "view-grid"],
    Decades: [href("decades"), "calendar"],
    Favourites: [href("favorites"), "seal"],
    History: [href("history"), "clock-rotate-right"],
  };

  /** The places the library keeps, beside a library page's title as Android's beside Library's. */
  export const PLACES = ["History"];
</script>

<script lang="ts">
  // The top bar on a wide screen, as the desktop's top pill: one outline holding Home and the library's
  // sections in their order (the core's `library_sections`), the current one in the accent and named
  // while there is room; then the search and the settings button, which opens Settings.
  import { ask } from "../rpc";
  import { remote } from "../state/remote.svelte";
  import { route } from "../state/route.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";
  import SearchBox from "./SearchBox.svelte";

  /** The page each section's tab is lit on, and stays lit on for the pages opened from it. */
  const tops: Record<string, string[]> = { Albums: ["albums"], Artists: ["artists"], Songs: ["songs"], Playlists: ["playlists"], Favourites: ["favorites"] };
  let sections = $state<string[]>([]);
  $effect(() => {
    void remote.jam?.guest;
    // Favorites last, as the desktop's top pill has it.
    void ask<string>("library_sections").then((s) => (sections = (JSON.parse(s) as string[]).filter((x) => x in tops).sort((a, b) => +(a === "Favourites") - +(b === "Favourites"))));
  });
  const on = (...t: string[]) => t.includes(route.from);
</script>

<header class="top chrome" id="nav">
  <nav class="tabs" aria-label={W.library}>
    <a class="tab" class:on={on("home", "row")} href={href("home")} data-top="home"><Icon name="home-simple" /><span class="name" data-name={W.home}>{W.home}</span></a>
    <span class="sep"></span>
    {#each sections as s (s)}
      <a class="tab" class:on={on(...tops[s])} href={sectionPage[s][0]} data-top={s === "Albums" ? "library" : undefined}><Icon name={sectionPage[s][1]} /><span class="name" data-name={W.librarySections[s]}>{W.librarySections[s]}</span></a>
    {/each}
  </nav>
  <SearchBox />
  <a class="glyph quiet gear" class:on={on("settings", "equalizer")} aria-label={W.settings} title={W.settings} data-top="settings" href={href("settings")}><Icon name="settings" /></a>
</header>

<style>
  .top {
    position: fixed;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    width: var(--pill-w);
    height: 48px;
    z-index: 30;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 8px;
    border-radius: 24px;
    border: 1px solid var(--outline);
  }
  .tabs {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 36px;
    padding: 0 12px;
    border-radius: 18px;
    color: var(--ink2);
    font-size: 13px;
    white-space: nowrap;
  }
  .tab :global(.icon) {
    width: 20px;
    height: 20px;
  }
  .tab:hover {
    background: var(--hover);
    color: var(--ink);
  }
  .tab.on {
    color: var(--accent);
    font-weight: 700;
  }
  /* The bold name's room, held by an unseen copy, so nothing moves as a tab lights. */
  .name {
    display: inline-grid;
  }
  .name::after {
    content: attr(data-name);
    font-weight: 700;
    height: 0;
    visibility: hidden;
  }
  .sep {
    width: 1px;
    height: 20px;
    background: var(--hair);
    margin: 0 4px;
  }
  .gear {
    width: 36px;
    height: 36px;
    margin-left: 6px;
  }
  .gear :global(.icon) {
    width: 20px;
    height: 20px;
  }
  /* Named only while there is room (the window under 912 px), as the desktop's; the current one always. */
  @media (max-width: 911px) {
    .tab span {
      display: none;
    }
    .tab.on span {
      display: inline;
    }
  }
</style>
