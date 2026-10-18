<script lang="ts">
  // The sidebar, as the desktop app has it: search, Home, the library's sections, the playlists, and
  // Settings at the foot.
  import { library } from "../state/library.svelte";
  import { remote } from "../state/remote.svelte";
  import { href, route, searchFor } from "../state/route.svelte";
  import { newPlaylist } from "../state/menus";
  import { W } from "../words";
  import Icon from "./Icon.svelte";
  import Cover from "./Cover.svelte";

  const guest = $derived(!!remote.jam?.guest);
  let query = $state(route.top === "search" ? route.arg : "");
  const on = (...tops: string[]) => tops.includes(route.top);
</script>

<nav class="side" id="nav" aria-label={W.library}>
  <div class="brand">{W.appName}</div>
  <form class="search" role="search" onsubmit={(e) => (e.preventDefault(), searchFor(query))}>
    <Icon name="search" />
    <input type="search" placeholder={W.search} aria-label={W.search} bind:value={query} onfocus={() => searchFor(query)} oninput={() => searchFor(query)} />
  </form>
  <div class="scroll">
    <a class="item" class:on={on("home")} href={href("home")} data-top="home"><Icon name="home" /><span>{W.home}</span></a>
    <h3>{W.library}</h3>
    <a class="item" class:on={on("artists", "artist")} href={href("artists")} data-top="library"><Icon name="artist" /><span>{W.artists}</span></a>
    <a class="item" class:on={on("albums", "album")} href={href("albums", "alphabeticalByName")}><Icon name="album" /><span>{W.albums}</span></a>
    <a class="item" class:on={on("songs")} href={href("songs")}><Icon name="note" /><span>{W.songs}</span></a>
    <a class="item" class:on={on("genres", "genre")} href={href("genres")}><Icon name="genre" /><span>{W.genres}</span></a>
    {#if !guest}
      <a class="item" class:on={on("favorites")} href={href("favorites")}><Icon name="heart" /><span>{W.favorites}</span></a>
      <h3>{W.playlists}<button class="add" title={W.newPlaylist} aria-label={W.newPlaylist} onclick={() => void newPlaylist()}><Icon name="plus" /></button></h3>
      <a class="item" class:on={on("playlists")} href={href("playlists")}><Icon name="playlist" /><span>{W.allPlaylists}</span></a>
      {#each library.playlists as p (p.id)}
        <a class="item pl" class:on={route.top === "playlist" && route.arg === p.id} href={href("playlist", p.id)}>
          <span class="art"><Cover src={p.cover} fallback="playlist" /></span><span class="clip">{p.name}</span>
        </a>
      {/each}
    {/if}
  </div>
  <div class="foot">
    <a class="item" class:on={on("settings")} href={href("settings")} data-top="settings"><Icon name="settings" /><span>{W.settings}</span></a>
  </div>
</nav>

<style>
  .side {
    grid-row: 1;
    background: var(--side);
    border-right: 1px solid var(--hair);
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 18px 12px 10px;
  }
  .brand {
    font: 800 22px/1 var(--display);
    letter-spacing: -0.03em;
    padding: 2px 10px 14px;
    color: var(--accent);
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 10px;
    border-radius: 8px;
    background: var(--fill);
    color: var(--dim);
    margin-bottom: 14px;
  }
  .search input {
    border: 0;
    outline: 0;
    background: none;
    width: 100%;
    font-size: 14px;
    color: var(--text);
  }
  .search :global(.icon) {
    width: 16px;
    height: 16px;
  }
  .scroll {
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }
  h3 {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font: 600 11px/1 var(--font);
    letter-spacing: 0.02em;
    color: var(--faint);
    padding: 18px 10px 6px;
    text-transform: uppercase;
  }
  .add {
    color: var(--dim);
  }
  .add :global(.icon) {
    width: 16px;
    height: 16px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 34px;
    padding: 0 10px;
    border-radius: 8px;
    font-size: 14px;
    font-weight: 500;
  }
  .item :global(.icon) {
    color: var(--accent);
    width: 20px;
    height: 20px;
  }
  .item:hover {
    background: var(--hover);
  }
  .item.on {
    background: var(--chosen);
  }
  .pl {
    font-weight: 400;
  }
  .art {
    width: 22px;
    flex: none;
  }
  .art :global(.cover) {
    border-radius: 4px;
  }
  .foot {
    border-top: 1px solid var(--hair);
    padding-top: 8px;
  }
</style>
