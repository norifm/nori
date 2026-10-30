<script lang="ts" module>
  import { href } from "../state/route.svelte";

  /** A library section's page and glyph, by the core's `LibrarySection`. */
  export const sectionPage: Record<string, [string, string]> = {
    Albums: [href("albums"), "compact-disc"],
    Artists: [href("artists"), "user"],
    Songs: [href("songs"), "music-note"],
    Playlists: [href("playlists"), "playlist"],
    Genres: [href("genres"), "view-grid"],
    Favourites: [href("favorites"), "seal"],
  };
</script>

<script lang="ts">
  // The top bar on a wide screen, as the desktop's top pill: one outline holding Home and the library's
  // sections in their order (the core's `library_sections`), the current one in the accent and named
  // while there is room; then the search and the settings button. Settings opens the account, the
  // equalizer and Settings, and the season and appearance. The wordmark stands left of it where it fits.
  import { ask } from "../rpc";
  import { remote } from "../state/remote.svelte";
  import { go, route } from "../state/route.svelte";
  import { W } from "../words";
  import wordmark from "../../../docs/brand/nori-wordmark.svg?raw";
  import Icon from "./Icon.svelte";
  import Look from "./Look.svelte";
  import SearchBox from "./SearchBox.svelte";

  /** The page each section's tab is lit on. */
  const tops: Record<string, string[]> = { Albums: ["albums", "album"], Artists: ["artists", "artist"], Songs: ["songs"], Playlists: ["playlists", "playlist"], Favourites: ["favorites"] };
  let sections = $state<string[]>([]);
  $effect(() => {
    void remote.jam?.guest;
    void ask<string>("library_sections").then((s) => (sections = (JSON.parse(s) as string[]).filter((x) => x in tops)));
  });
  let pop = $state(false);
  let bar = $state<HTMLElement>();
  let account = $state<{ url: string; user: string } | null>(null);
  const on = (...t: string[]) => t.includes(route.top);
  const toggle = () => {
    pop = !pop;
    if (pop && !account) void ask<string>("profile").then((j) => (account = JSON.parse(j)));
  };
  const open = (page: string) => {
    go(page);
    pop = false;
  };
</script>

<svelte:window onpointerdown={(e) => pop && !bar?.contains(e.target as Node) && (pop = false)} onkeydown={(e) => e.key === "Escape" && (pop = false)} onhashchange={() => (pop = false)} />

<a class="brand" href={href("home")} aria-label={W.appName}>{@html wordmark}</a>
<header class="top chrome" id="nav" bind:this={bar}>
  <nav class="tabs" aria-label={W.library}>
    <a class="tab" class:on={on("home", "row")} href={href("home")} data-top="home"><Icon name="home-simple" /><span>{W.home}</span></a>
    <span class="sep"></span>
    {#each sections as s (s)}
      <a class="tab" class:on={on(...tops[s])} href={sectionPage[s][0]} data-top={s === "Albums" ? "library" : undefined}><Icon name={sectionPage[s][1]} /><span>{W.librarySections[s]}</span></a>
    {/each}
  </nav>
  <SearchBox />
  <button class="glyph quiet gear" class:on={pop || on("settings", "equalizer")} aria-label={W.settings} title={W.settings} aria-expanded={pop} data-top="settings" onclick={toggle}><Icon name="settings" /></button>
  {#if pop}
    <div class="pop popover">
      <div class="acct">
        <span class="initial">{(account?.user ?? "").slice(0, 1).toUpperCase()}</span>
        <span class="who clip">{account?.user ?? ""}<span class="sub clip">{account?.url ?? ""}</span></span>
      </div>
      <button class="popover-item" onclick={() => open("equalizer")}><Icon name="control-slider" /><span class="clip">{W.equalizer}</span></button>
      <button class="popover-item" onclick={() => open("settings")}><Icon name="settings" /><span class="clip">{W.settings}</span></button>
      <Look />
    </div>
  {/if}
</header>

<style>
  .top {
    position: fixed;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    width: min(1040px, calc(100% - 32px));
    height: 48px;
    z-index: 30;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 8px;
    border-radius: 24px;
    border: 1px solid var(--outline);
  }
  /* The wordmark in the page's ink, its record's face the page's paper; its left edge on the page's,
     only where it clears the pill. */
  .brand {
    --logo-ink: var(--ink);
    --logo-rice: var(--page);
    position: fixed;
    top: 23px;
    left: var(--gutter);
    z-index: 30;
  }
  .brand :global(svg) {
    display: block;
    height: 26px;
    width: auto;
  }
  @media (max-width: 1279px) {
    .brand {
      display: none;
    }
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
    font-weight: 500;
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
  .pop {
    position: absolute;
    top: 56px;
    right: 0;
    width: 300px;
  }
  .acct {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 6px 12px;
    border-bottom: var(--hairline) solid var(--hair);
    margin-bottom: 6px;
  }
  .initial {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    flex: none;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 700;
  }
  .who {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-weight: 600;
  }
  .pop :global(.look) {
    margin-top: 6px;
    padding: 6px 4px 4px;
    border-top: var(--hairline) solid var(--hair);
  }
  /* Named only while there is room, as the desktop's; the current one always. */
  @media (max-width: 1023px) {
    .tab span {
      display: none;
    }
    .tab.on span {
      display: inline;
    }
  }
</style>
