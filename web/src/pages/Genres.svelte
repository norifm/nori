<script lang="ts">
  // Every genre with songs, by name, each with a cover of its songs (the core's `genres_listed`).
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { href } from "../state/route.svelte";
  import type { Genre } from "../state/types";
  import { W } from "../words";
  import Cover from "../ui/Cover.svelte";
  import Page from "../ui/Page.svelte";

  const page = reading("genres", () => read<{ genres: Genre[] }>({ kind: "genres" }).then((p) => p.genres ?? []));
</script>

<Page title={W.librarySections.Genres} back places ready={!!page.data} error={page.error}>
  <ul class="genres">
    {#each page.data ?? [] as g (g.name)}
      <li><a href={href("genre", g.name)}><span class="art"><Cover src={g.cover} radius="calc(var(--r-tile) - 12px)" /></span><span class="name">{g.name}</span><span class="sub">{W.songCount(g.songCount)}</span></a></li>
    {/each}
  </ul>
</Page>

<style>
  .genres {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--grid-gap);
  }
  a {
    display: grid;
    grid-template-columns: 64px 1fr;
    grid-template-rows: 1fr 1fr;
    column-gap: 14px;
    align-items: center;
    height: 88px;
    padding: 12px;
    border-radius: var(--r-tile);
    background: var(--surface);
    border: var(--hairline) solid var(--hair);
  }
  a:hover {
    background: color-mix(in srgb, var(--ink) 5%, var(--surface));
  }
  .art {
    grid-row: 1 / 3;
    width: 64px;
    height: 64px;
  }
  .sub {
    align-self: start;
  }
  .name {
    align-self: end;
    font-size: 17px;
    font-weight: 600;
    letter-spacing: -0.2px;
  }
</style>
