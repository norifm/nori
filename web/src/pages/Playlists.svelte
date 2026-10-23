<script lang="ts">
  // The Library's playlists: its places, New playlist, then each playlist with its cover, how many songs
  // and how long, and a way to delete it.
  import type { Snippet } from "svelte";
  import { read } from "../rpc";
  import { library, reading } from "../state/library.svelte";
  import { newPlaylist } from "../state/menus";
  import { href } from "../state/route.svelte";
  import type { Playlist } from "../state/types";
  import { W } from "../words";
  import Cover from "../ui/Cover.svelte";
  import Icon from "../ui/Icon.svelte";

  let { places }: { places: Snippet } = $props();
  const page = reading("library", () => read<{ playlists: Playlist[] }>({ kind: "playlists" }).then((p) => p.playlists ?? []));

  async function remove(p: Playlist) {
    if (confirm(W.deleteConfirm(p.name))) await library.change({ kind: "deletePlaylist", id: p.id });
  }
</script>

{@render places()}
<ul class="rows">
  <li><button class="nav-row glyphed" id="new-playlist" onclick={() => void newPlaylist()}><Icon name="plus" /><span class="words">{W.newPlaylist}</span></button></li>
  {#each page.data ?? [] as p (p.id)}
    <li class="playlist">
      <a class="nav-row pictured" href={href("playlist", p.id)}>
        <span class="picture"><Cover src={p.cover} fallback="playlist" /></span>
        <span class="words"><span>{p.name}</span><span class="sub">{W.listCaption(p.songCount, p.duration)}</span></span>
      </a>
      <button class="more" aria-label={W.deletePlaylist} title={W.deletePlaylist} onclick={() => void remove(p)}><Icon name="trash" /></button>
    </li>
  {/each}
</ul>
{#if page.error}<p class="empty">{W.failed(page.error)}</p>
{:else if page.data && !page.data.length}<p class="empty"><Icon name="playlist" />{W.noPlaylists}</p>{/if}

<style>
  .playlist {
    position: relative;
  }
  .playlist .nav-row {
    padding-right: 64px;
  }
  .more {
    position: absolute;
    right: 12px;
    top: 50%;
    translate: 0 -50%;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    color: var(--ink2);
  }
  .more :global(.icon) {
    width: 19px;
    height: 19px;
  }
  .more:hover {
    background: var(--hover);
  }
</style>
