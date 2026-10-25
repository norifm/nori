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
  // The top bar on a wide screen: one outline with Home and the library's sections in the order kept (the
  // core's `library_sections`), the current one in the accent; the search field and the settings button
  // beside it. Settings opens the account, the season and appearance, Genres, Play on and Settings.
  import { ask } from "../rpc";
  import { remote } from "../state/remote.svelte";
  import { go, route } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
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
  let pop = $state<"settings" | null>(null);
  let bar = $state<HTMLElement>();
  let account = $state<{ url: string; user: string } | null>(null);
  const on = (...t: string[]) => t.includes(route.top);
  const toggle = () => {
    pop = pop ? null : "settings";
    if (pop && !account) void ask<string>("profile").then((j) => (account = JSON.parse(j)));
  };

  async function signOut() {
    await ask("logout");
    location.hash = "";
    location.reload();
  }
</script>

<svelte:window onpointerdown={(e) => pop && !bar?.contains(e.target as Node) && (pop = null)} onkeydown={(e) => e.key === "Escape" && (pop = null)} onhashchange={() => (pop = null)} />

<header class="top" bind:this={bar}>
  <a class="brand" href={href("home")}>{W.appName}</a>
  <nav class="chrome tabs" id="nav" aria-label={W.library}>
    <a class="tab" class:on={on("home", "row")} href={href("home")} data-top="home"><Icon name="home-simple" /><span>{W.home}</span></a>
    <span class="sep"></span>
    {#each sections as s (s)}
      <a class="tab" class:on={on(...tops[s])} href={sectionPage[s][0]} data-top={s === "Albums" ? "library" : undefined}><Icon name={sectionPage[s][1]} /><span>{W.librarySections[s]}</span></a>
    {/each}
  </nav>
  <div class="right">
    <SearchBox />
    <button class="chrome gear" class:open={pop === "settings"} aria-label={W.settings} title={W.settings} aria-expanded={pop === "settings"} data-top="settings" onclick={toggle}><Icon name="settings" /></button>
  </div>
  {#if pop === "settings"}
    <div class="pop popover">
      <div class="acct">
        <span class="who clip">{account?.user ?? ""}<span class="sub clip">{account?.url ?? ""}</span></span>
        <button class="pill small quiet" onclick={() => void signOut()}>{W.signOut}</button>
      </div>
      <Look />
      <a class="popover-item" href={href("genres")}><Icon name="view-grid" /><span class="clip">{W.librarySections.Genres}</span></a>
      {#if remote.devices.on}
        <button class="popover-item" onclick={() => (ui.show("devices"), (pop = null))}><Icon name="computer" /><span class="clip">{W.devices}</span></button>
      {/if}
      <button class="popover-item" onclick={() => (go("settings"), (pop = null))}><Icon name="settings" /><span class="clip">{W.settings}</span></button>
    </div>
  {/if}
</header>

<style>
  .top {
    position: fixed;
    top: 14px;
    left: 16px;
    right: 16px;
    z-index: 30;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: 12px;
    height: 50px;
  }
  .brand {
    font: 700 22px/1 var(--display);
    letter-spacing: -0.3px;
    padding-left: 6px;
    justify-self: start;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 0 6px;
    height: 50px;
    border-radius: 25px;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 13px;
    color: color-mix(in srgb, var(--ink) 65%, transparent);
    font-size: 14px;
    font-weight: 500;
    white-space: nowrap;
    transition: color 220ms;
  }
  .tab :global(.icon) {
    width: 21px;
    height: 21px;
  }
  .tab:hover {
    color: var(--ink);
  }
  .tab.on {
    color: var(--accent);
  }
  .sep {
    width: 1px;
    background: var(--chrome);
    margin: 14px 6px;
  }
  .right {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 8px;
  }
  .gear {
    width: 50px;
    height: 50px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: color-mix(in srgb, var(--ink) 65%, transparent);
  }
  .gear :global(.icon) {
    width: 21px;
    height: 21px;
  }
  .gear:hover {
    color: var(--ink);
  }
  .gear.open {
    color: var(--accent);
  }
  .pop {
    position: absolute;
    top: 58px;
    right: 0;
    width: 360px;
  }
  .acct {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 6px 4px 12px 10px;
    border-bottom: var(--hairline) solid var(--hair);
    margin-bottom: 6px;
  }
  .who {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-weight: 600;
  }
  .pop :global(.look) {
    padding: 0 4px 10px;
    border-bottom: var(--hairline) solid var(--hair);
    margin-bottom: 4px;
  }
  @media (max-width: 1023px) {
    .tab span {
      display: none;
    }
    .tab.on span {
      display: inline;
    }
    .tab {
      padding: 0 11px;
    }
  }
</style>
