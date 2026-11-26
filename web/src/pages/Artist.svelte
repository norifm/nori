<script lang="ts">
  // An artist, as the desktop's page: the picture across the head with the name, Play and Shuffle (every
  // album of theirs) over its foot; the list's head ("14 releases · 162 songs" and the page's glyphs); its
  // releases by kind as the core grouped them (`release_groups`), a shelf each; then the songs its provider
  // lists, and its about part (the core's `ArtistAbout`): the biography with where to read on, and the similar
  // artists at its foot. Its head shows at once from what the card that opened it knew (`known`).
  import { ask, read } from "../rpc";
  import { known } from "../state/known";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import type { Album, Artist, Song } from "../state/types";
  import { W } from "../words";
  import ArtistHead from "../ui/ArtistHead.svelte";
  import Cards from "../ui/Cards.svelte";
  import Page from "../ui/Page.svelte";
  import PageTools from "../ui/PageTools.svelte";
  import Seal from "../ui/Seal.svelte";
  import SongList from "../ui/SongList.svelte";

  let { id }: { id: string } = $props();
  type Group = { kind: string; tag: string; albums: number[] };
  /** The core's `ArtistAbout`. */
  type About = { biography: string | null; lastFm: string | null; musicbrainz: string | null; similar: Artist[] };
  let aboutRead = $state.raw<About & { id: string }>();
  const page = reading("artist", async () => {
    const d = await read<{ artist: Artist; picture?: string; albums: Album[]; songs: Song[]; few: number; groups: Group[] }>({ kind: "artist", id });
    // After the page: a provider answers both from one read of the artist. A page without it is still the page.
    const asked = id;
    void read<About>({ kind: "artistAbout", id: asked }).then((a) => (aboutRead = { ...a, id: asked }), () => {});
    return { ...d, tools: JSON.parse(await ask<string>("page_actions", "Artist", d.songs.length)) as string[] };
  });
  // The answer while it is this page's: the page stays mounted when another artist opens over it.
  const data = $derived(page.data?.artist.id === id ? page.data : undefined);
  // Until the artist is read, what the card that opened the page knew.
  const a = $derived(data?.artist ?? known("artist", id));
  const albums = $derived(data?.albums ?? []);
  const songs = $derived(data?.songs ?? []);
  const shelves = $derived((data?.groups ?? []).map((g) => ({ name: W.shelf(g.kind, g.tag), albums: g.albums.map((i) => albums[i]) })));
  const about = $derived(aboutRead?.id === id ? aboutRead : undefined);
  const links = $derived([about?.lastFm && { name: W.lastFm, url: about.lastFm }, about?.musicbrainz && { name: W.musicBrainz, url: about.musicbrainz }].filter((l) => !!l));
  // The biography four lines long until More, which shows only when there is more.
  let bio = $state<HTMLElement>();
  let open = $state(false);
  let longer = $state(false);
  $effect(() => {
    void about?.biography;
    open = false;
    longer = !!bio && bio.scrollHeight > bio.clientHeight + 1;
  });
  const label = $derived.by(() => {
    if (!data) return a?.albumCount ? W.releases(a.albumCount) : "";
    if (!albums.length) return songs.length ? W.songCount(songs.length) : "";
    const n = albums.reduce((s, x) => s + (x.songCount || 0), 0);
    return [W.releases(albums.length), n ? W.songCount(n) : ""].filter(Boolean).join(" · ");
  });
</script>

<Page back ready={!!a} error={page.error}>
  {#if a}
    <ArtistHead id={a.id} name={a.name} cover={a.cover} picture={data?.picture} onplay={(s) => void player.playWhole("artist", id, s)}>
      {#snippet actions()}
        {#if !remote.jam?.guest}<Seal what="artist" id={a.id} on={!!a.starred} />{/if}
      {/snippet}
    </ArtistHead>
    <div class="list-head">
      <span class="caption">{label}</span>
      {#if data}<PageTools tools={data.tools} artist={a.id} />{/if}
    </div>
    {#if about?.biography || links.length}
      <div class="about">
        {#if about?.biography}<p class="bio" class:open bind:this={bio}>{about.biography}</p>{/if}
        <div class="links">
          {#if longer || open}<button class="text-action" onclick={() => (open = !open)}>{open ? W.less : W.more}</button>{/if}
          {#each links as l (l.url)}<a class="text-action" href={l.url} target="_blank" rel="noopener">{l.name}</a>{/each}
        </div>
      </div>
    {/if}
    {#each shelves as s, n (n)}
      <h2 class="section">{s.name}</h2>
      <Cards albums={s.albums} row />
    {/each}
    {#if songs.length}<h2 class="section">{W.songs}</h2><SongList {songs} kind="Artist" id={a.id} shown={data?.few} />{/if}
    {#if about?.similar.length}<h2 class="section">{W.similarArtists}</h2><Cards artists={about.similar} row />{/if}
  {/if}
</Page>

<style>
  /* What the artist has and the page's glyphs, a hairline under, as the desktop's list head. */
  .list-head {
    display: flex;
    align-items: center;
    gap: 2px;
    min-height: 48px;
    margin-top: 18px;
    border-bottom: 1px solid var(--line);
  }
  /* The biography at a reading width, four lines until More; then where to read on. */
  .about {
    margin-top: 16px;
    max-width: 760px;
  }
  .bio {
    font-size: 15px;
    line-height: 22px;
    white-space: pre-line;
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .bio.open {
    display: block;
  }
  .links {
    display: flex;
    flex-wrap: wrap;
    margin: 4px 0 0 -12px;
  }
  .links a {
    display: inline-flex;
    align-items: center;
  }
  .caption {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    color: var(--ink2);
  }
</style>
