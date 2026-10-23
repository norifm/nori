<script lang="ts">
  // The library, as Android's: its sections as chips in the order kept (the core's `library_sections`), the
  // one chosen below them (the address names it), the places at the top of each: Favorites, and Shuffle
  // where the section has one.
  import type { Component } from "svelte";
  import { ask } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import { go, href, route } from "../state/route.svelte";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";
  import Albums from "./Albums.svelte";
  import Artists from "./Artists.svelte";
  import Genres from "./Genres.svelte";
  import Playlists from "./Playlists.svelte";
  import Songs from "./Songs.svelte";

  const drawn: Record<string, Component<any>> = { Albums, Artists, Songs, Playlists, Genres };
  const sections = reading("library", async () => JSON.parse(await ask<string>("library_sections")) as string[]);
  const chips = $derived((sections.data ?? []).filter((s) => s in drawn));
  const shown = $derived(chips.includes(route.arg) ? route.arg : chips[0]);
  const favorites = $derived(!!sections.data?.includes("Favourites"));
  /** The section's Shuffle: random albums, or random songs; the account's own library only. */
  const shuffle = $derived(remote.jam?.guest ? null : shown === "Albums" ? () => player.shuffleLibrary(true) : shown === "Songs" ? () => player.shuffleLibrary(false) : null);
  const Section = $derived(shown ? drawn[shown] : null);
</script>

<Page title={W.library} ready={!!sections.data} error={sections.error}>
  <div class="chips" role="tablist">
    {#each chips as s (s)}
      <button class="chip" class:on={s === shown} role="tab" aria-selected={s === shown} data-section={s} onclick={() => go("library", s)}>{W.librarySections[s]}</button>
    {/each}
  </div>
  {#if Section}<Section {places} />{/if}
</Page>

{#snippet places()}
  {#if favorites || shuffle}
    <div class="group places">
      {#if favorites}<a class="group-row" href={href("favorites")}><Icon name="seal" /><span class="words">{W.favorites}</span><Icon name="nav-arrow-right" /></a>{/if}
      {#if shuffle}<button class="group-row" onclick={() => void shuffle()}><Icon name="shuffle" /><span class="words">{W.shuffle}</span></button>{/if}
    </div>
  {/if}
{/snippet}

<style>
  .places {
    margin: 0 -4px 10px;
  }
</style>
