<script lang="ts">
  // One of Home's rows in full, as the core pages it (`home_row_page`): first what its shelf shows, in its
  // order, then a page after another once that is in; or its playlists, whole.
  import { ask } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { route } from "../state/route.svelte";
  import type { Album, Playlist } from "../state/types";
  import { W } from "../words";
  import Cards from "../ui/Cards.svelte";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";

  type RowPage = { albums?: Album[]; playlists?: Playlist[]; more: boolean } | null;
  const name = $derived(route.arg);
  const page = (offset: number) => ask<string>("home_row_page", route.arg, offset).then((p) => JSON.parse(p) as RowPage);
  const first = reading("row", () => page(0));
  /** The pages after the first. */
  let rest = $state.raw<Album[]>([]);
  $effect(() => {
    const p = first.data;
    rest = [];
    let gone = false;
    void (async () => {
      let offset = p?.albums?.length ?? 0;
      // A page that fails ends the reading here; the row shown again reads it again.
      for (let more = p?.more; more && !gone; ) {
        const next = await page(offset).catch(() => null);
        if (gone || !next?.albums) return;
        rest = [...rest, ...next.albums];
        offset += next.albums.length;
        more = next.more;
      }
    })();
    return () => {
      gone = true;
    };
  });
  const albums = $derived([...(first.data?.albums ?? []), ...rest]);
</script>

<Page title={W.homeRows[name] ?? name} back sketch="Albums" ready={!!first.data || first.data === null} error={first.error}>
  {#if first.data?.playlists}<Cards playlists={first.data.playlists} />
  {:else if albums.length}<Cards {albums} />
  {:else if first.data !== undefined}<p class="empty"><Icon name="compact-disc" />{W.noAlbums}</p>{/if}
</Page>

<style>
  .empty {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--ink2);
  }
</style>
