<script lang="ts">
  // The library's sections (a phone's Library tab; the sidebar lists them on a wide screen), and what
  // was added last.
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { remote } from "../state/remote.svelte";
  import { href } from "../state/route.svelte";
  import type { Album } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";

  const page = reading("library", () => read<{ albums: Album[] }>({ kind: "albums", sort: "newest", size: 12, offset: 0 }).then((p) => p.albums ?? []));
  const sections = $derived(
    [
      [W.playlists, href("playlists"), "playlist"],
      [W.artists, href("artists"), "artist"],
      [W.albums, href("albums", "alphabeticalByName"), "album"],
      [W.songs, href("songs"), "note"],
      [W.genres, href("genres"), "genre"],
      [W.favorites, href("favorites"), "heart"],
    ].filter(([, h]) => !remote.jam?.guest || !(h === href("playlists") || h === href("favorites"))),
  );
</script>

<Page title={W.library} ready={!!page.data} error={page.error}>
  <ul class="sections">
    {#each sections as [name, h, icon] (h)}
      <li><a href={h}><Icon name={icon} /><span>{name}</span><Icon name="chevron" /></a></li>
    {/each}
  </ul>
  <h2 class="section">{W.recentlyAdded}</h2>
  <Cards albums={page.data ?? []} />
</Page>

<style>
  .sections {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .sections a {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 48px;
    border-bottom: 1px solid var(--hair);
    font-size: 19px;
  }
  .sections a span {
    flex: 1;
  }
  .sections :global(.icon) {
    color: var(--accent);
    width: 24px;
    height: 24px;
  }
  .sections :global(.icon:last-child) {
    color: var(--faint);
    width: 20px;
  }
</style>
