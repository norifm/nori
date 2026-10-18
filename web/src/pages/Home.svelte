<script lang="ts">
  // Home: shelves of the newest, recently and most played albums, and a few at random.
  import { facts, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { href } from "../state/route.svelte";
  import type { Album } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Page from "../ui/Page.svelte";

  const list = (sort: string, size = 12) => read<{ albums: Album[] }>({ kind: "albums", sort, size, offset: 0 }).then((p) => p.albums ?? [], () => []);
  const page = reading("home", async () => {
    const [newest, recent, frequent, random] = await Promise.all([list("newest"), list("recent"), list("frequent"), list("random", 12)]);
    facts.albums = newest.length;
    return [
      [W.recentlyAdded, "newest", newest],
      [W.recentlyPlayed, "recent", recent],
      [W.mostPlayed, "frequent", frequent],
      [W.discover, "random", random],
    ] as [string, string, Album[]][];
  });
</script>

<Page title={W.home} ready={!!page.data} error={page.error}>
  {#each page.data ?? [] as [name, sort, albums] (sort)}
    {#if albums.length}
      <section class="shelf">
        <div class="shelf-head"><h2>{name}</h2><a href={href("albums", sort)}>{W.seeAll}</a></div>
        <Cards {albums} row />
      </section>
    {/if}
  {/each}
</Page>

<style>
  .shelf {
    margin-bottom: 30px;
  }
  .shelf-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: 12px;
  }
  h2 {
    font-size: 21px;
    font-weight: 700;
  }
  .shelf-head a {
    color: var(--accent);
    font-size: 14px;
  }
</style>
