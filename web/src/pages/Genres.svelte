<script lang="ts">
  // The Library's genres: its places, then every genre with songs, by name, with how many.
  import type { Snippet } from "svelte";
  import { read } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { href } from "../state/route.svelte";
  import type { Genre } from "../state/types";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";

  let { places }: { places: Snippet } = $props();
  const page = reading("library", () =>
    read<{ genres: Genre[] }>({ kind: "genres" }).then((p) => (p.genres ?? []).filter((g) => g.songCount > 0).sort((a, b) => a.value.localeCompare(b.value))),
  );
</script>

{@render places()}
{#if page.error}<p class="empty">{W.failed(page.error)}</p>{/if}
<ul class="rows genres">
  {#each page.data ?? [] as g (g.value)}
    <li><a class="nav-row" href={href("genre", g.value)}><span class="words">{g.value}</span><span class="value">{g.songCount}</span><Icon name="nav-arrow-right" /></a></li>
  {/each}
</ul>
