<script lang="ts">
  // The decades with songs in the index, newest first, as Android's list: each its songs counted, opening its
  // songs (the core's `browse_decades`).
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { href } from "../state/route.svelte";
  import type { Decade } from "../state/types";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";
  import Lines from "../ui/Lines.svelte";
  import Page from "../ui/Page.svelte";

  const page = reading("decades", () => read<{ decades: Decade[] }>({ kind: "decades" }).then((p) => p.decades ?? []));
</script>

<Page title={W.librarySections.Decades} back narrow ready={!!page.data} error={page.error}>
  {#if page.data?.length}
    <Lines lines={page.data.map((d) => ({ key: String(d.start), href: href("decade", String(d.start)), name: W.decade(d.start), count: d.songCount }))} />
  {:else if page.data}<p class="empty"><Icon name="music-note" />{W.noIndex}</p>{/if}
</Page>
