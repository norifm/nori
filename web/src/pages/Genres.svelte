<script lang="ts">
  // Every genre with songs, by name, as Android's list: each its songs counted (the core's `genres_listed`).
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { href } from "../state/route.svelte";
  import type { Genre } from "../state/types";
  import { W } from "../words";
  import Lines from "../ui/Lines.svelte";
  import Page from "../ui/Page.svelte";

  const page = reading("genres", () => read<{ genres: Genre[] }>({ kind: "genres" }).then((p) => p.genres ?? []));
</script>

<Page title={W.librarySections.Genres} back narrow ready={!!page.data} error={page.error}>
  <Lines lines={(page.data ?? []).map((g) => ({ key: g.name, href: href("genre", g.name), name: g.name, count: g.songCount }))} />
</Page>
