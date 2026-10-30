<script lang="ts">
  // The library's sections (a phone's Library tab; the top bar lists them on a wide screen), in their
  // order (the core's `library_sections`), and what was added last.
  import { ask, read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import type { Album } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import { sectionPage } from "../ui/TopPill.svelte";

  const page = reading("library", async () => {
    const [sections, albums] = await Promise.all([
      ask<string>("library_sections").then((s) => JSON.parse(s) as string[]),
      read<{ albums: Album[] }>({ kind: "albums", sort: "newest", size: 12, offset: 0 }).then((p) => p.albums ?? []),
    ]);
    return { sections: sections.filter((s) => s in sectionPage), albums };
  });
</script>

<Page title={W.library} ready={!!page.data} error={page.error}>
  <div class="group sections">
    {#each page.data?.sections ?? [] as s (s)}
      {@const [h, icon] = sectionPage[s]}
      <a class="group-row" href={h}><Icon name={icon} /><span class="words">{W.librarySections[s]}</span><Icon name="nav-arrow-right" /></a>
    {/each}
  </div>
  <h2 class="section">{W.homeRows.Newest}</h2>
  <Cards albums={page.data?.albums ?? []} />
</Page>
