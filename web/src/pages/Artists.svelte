<script lang="ts">
  // Every artist.
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import type { Artist } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Page from "../ui/Page.svelte";

  const page = reading("artists", () => read<{ artists: Artist[] }>({ kind: "artists" }).then((p) => p.artists ?? []));
</script>

<Page title={W.artists} back ready={!!page.data} error={page.error}>
  <Cards artists={page.data ?? []} />
</Page>
