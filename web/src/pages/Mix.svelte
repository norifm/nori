<script lang="ts">
  // A mix from "For you": its songs as they play (the core's `mix_page`, drawn for this day or week if it
  // was not yet), under its name and what it is made from, with Play and Shuffle.
  import { ask } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import type { MixPage } from "../state/types";
  import { W } from "../words";
  import Hero from "../ui/Hero.svelte";
  import Page from "../ui/Page.svelte";
  import SongList from "../ui/SongList.svelte";

  let { id }: { id: string } = $props();
  const page = reading("mix", async () => JSON.parse(await ask<string>("mix_page", id)) as MixPage | null);
  const mix = $derived(page.data);
  const songs = $derived(mix?.songs ?? []);
</script>

<Page back ready={page.data !== undefined} error={page.error}>
  {#if mix}
    <Hero cover={mix.covers[0]} title={W.mixNames[mix.name] ?? mix.name} fallback="sparks" label={songs.length ? W.listCaption(songs.length, mix.seconds) : ""} onplay={(s) => void player.play(songs, 0, s, "Mix", id)}>
      {#snippet by()}{W.mixCaption(mix.favourites)}{/snippet}
    </Hero>
    <SongList {songs} kind="Mix" {id} />
  {:else}<p class="empty">{W.empty}</p>{/if}
</Page>
