<script lang="ts">
  // The top bar on a wide screen: one outline with Home, the library's sections and Favorites; the search
  // field and the settings button beside it. Settings opens the account, the season and the way to
  // Settings.
  import { ask } from "../rpc";
  import { remote } from "../state/remote.svelte";
  import { go, href, route } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";
  import Look from "./Look.svelte";
  import SearchBox from "./SearchBox.svelte";

  const guest = $derived(!!remote.jam?.guest);
  let pop = $state<"settings" | null>(null);
  let bar = $state<HTMLElement>();
  let account = $state<{ url: string; user: string } | null>(null);
  const on = (...tops: string[]) => tops.includes(route.top);
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
    <a class="tab" class:on={on("home")} href={href("home")} data-top="home"><Icon name="home" /><span>{W.home}</span></a>
    <span class="sep"></span>
    <a class="tab" class:on={on("albums", "album")} href={href("albums", "alphabeticalByName")}><Icon name="album" /><span>{W.albums}</span></a>
    <a class="tab" class:on={on("artists", "artist")} href={href("artists")} data-top="library"><Icon name="artist" /><span>{W.artists}</span></a>
    <a class="tab" class:on={on("songs")} href={href("songs")}><Icon name="note" /><span>{W.songs}</span></a>
    {#if !guest}
      <a class="tab" class:on={on("playlists", "playlist")} href={href("playlists")}><Icon name="playlist" /><span>{W.playlists}</span></a>
      <a class="tab" class:on={on("favorites")} href={href("favorites")}><Icon name={on("favorites") ? "seal" : "seal-line"} /><span>{W.favorites}</span></a>
    {/if}
  </nav>
  <div class="right">
    <SearchBox />
    <button class="chrome gear" class:open={pop === "settings"} aria-label={W.settings} title={W.settings} aria-expanded={pop === "settings"} data-top="settings" onclick={toggle}><Icon name="settings" /></button>
  </div>
  {#if pop === "settings"}
    <div class="pop chrome menu">
      <div class="acct">
        <span class="who clip">{account?.user ?? ""}<span class="sub clip">{account?.url ?? ""}</span></span>
        <button class="pill small" onclick={() => void signOut()}>{W.signOut}</button>
      </div>
      <Look />
      <a class="item" href={href("genres")}><Icon name="genre" /><span>{W.genres}</span></a>
      {#if remote.devices.on}
        <button class="item" onclick={() => (ui.show("devices"), (pop = null))}><Icon name="devices" /><span>{W.playOn}</span></button>
      {/if}
      <button class="item" onclick={() => (go("settings"), (pop = null))}><Icon name="settings" /><span>{W.settings}</span></button>
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
    /* The margins beside the bars take the clicks that land on them. */
    pointer-events: auto;
    height: 48px;
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
    padding: 4px;
    height: 48px;
    border-radius: 24px;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 14px;
    border-radius: 20px;
    color: var(--ink2);
    font-size: 14px;
    font-weight: 500;
    white-space: nowrap;
  }
  .tab :global(.icon) {
    width: 20px;
    height: 20px;
  }
  .tab:hover {
    color: var(--ink);
  }
  .tab.on {
    color: var(--accent);
    font-weight: 700;
  }
  .sep {
    width: 1px;
    background: var(--chrome);
    margin: 10px 6px;
  }
  .right {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 8px;
  }
  .gear {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: var(--ink2);
  }
  .gear:hover,
  .gear.open {
    color: var(--ink);
  }
  .pop {
    position: absolute;
    top: 56px;
    padding: 8px;
    border-radius: 20px;
    background: var(--surface);
    backdrop-filter: none;
    display: flex;
    flex-direction: column;
  }
  .menu {
    right: 0;
    width: 340px;
    padding: 14px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px;
    border-radius: 12px;
    font-size: 14px;
    text-align: left;
  }
  .item:hover {
    background: var(--hover);
  }
  .item :global(.icon) {
    width: 22px;
    height: 22px;
    color: var(--ink2);
  }
  .acct {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--hair);
    margin-bottom: 6px;
  }
  .who {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-weight: 600;
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
