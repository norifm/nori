<script lang="ts">
  // Every genre with songs, by name.
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { href } from "../state/route.svelte";
  import type { Genre } from "../state/types";
  import { W } from "../words";
  import Page from "../ui/Page.svelte";

  const page = reading("genres", () =>
    read<{ genres: Genre[] }>({ kind: "genres" }).then((p) => (p.genres ?? []).filter((g) => g.songCount > 0).sort((a, b) => a.value.localeCompare(b.value))),
  );
</script>

<Page title={W.librarySections.Genres} back ready={!!page.data} error={page.error}>
  <ul class="genres">
    {#each page.data ?? [] as g (g.value)}
      <li><a href={href("genre", g.value)}><span class="name">{g.value}</span><span class="sub">{W.songCount(g.songCount)}</span></a></li>
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
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    height: 96px;
    padding: 14px;
    border-radius: var(--r-tile);
    background: var(--surface);
    border: var(--hairline) solid var(--hair);
  }
  a:hover {
    background: color-mix(in srgb, var(--ink) 5%, var(--surface));
  }
  .name {
    font-size: 17px;
    font-weight: 600;
    letter-spacing: -0.2px;
  }
</style>
