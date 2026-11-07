<script lang="ts">
  // Adding songs to a playlist, as Spotify's picker: a field that finds a playlist as it is typed in, or
  // names a new one; the playlists under it, each saying what it has of the songs already (the core's
  // `playlist_choices`), a click adding the ones it lacks. One that has them all is checked and takes
  // nothing. New playlist with nothing typed asks for its name in the field, as the desktop's turns into one.
  import { library } from "../state/library.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import type { PlaylistChoice } from "../state/types";

  let dialog = $state<HTMLDialogElement>();
  const p = $derived(ui.picking!);
  let name = $state("");
  let field = $state<HTMLInputElement>();
  /** New playlist was pressed with nothing typed: the field asks for its name. */
  let naming = $state(false);
  const typed = $derived(name.trim());
  const found = $derived((p.choices ?? []).filter((c) => c.playlist.name.toLowerCase().includes(typed.toLowerCase())));
  const close = () => (ui.picking = null);
  $effect(() => dialog?.showModal());

  async function add(c: PlaylistChoice) {
    close();
    await library.change({ kind: "addToPlaylist", id: c.playlist.id, songIds: c.adds });
    ui.say(W.addedTo(c.playlist.name));
  }
  async function create() {
    if (!typed) {
      naming = true;
      field?.focus();
      return;
    }
    const [name, songIds] = [typed, p.songIds];
    close();
    await library.change({ kind: "createPlaylist", name, songIds });
    ui.say(W.addedTo(name));
  }
</script>

<dialog class="card" id="playlist-picker" bind:this={dialog} onclose={close}>
  <h2>{W.addToPlaylist}</h2>
  <form
    class="field"
    onsubmit={(e) => {
      e.preventDefault();
      void create();
    }}
  >
    <Icon name="search" />
    <input bind:this={field} bind:value={name} placeholder={naming ? W.playlistName : W.findOrName} aria-label={naming ? W.playlistName : W.findOrName} autocomplete="off" />
  </form>
  <div class="group">
    <button class="group-row row new" onclick={() => void create()}><Icon name="plus" /><span class="words">{typed ? W.newPlaylistNamed(typed) : W.newPlaylist}</span></button>
  </div>
  {#if found.length}
    <div class="group list">
      {#each found as c (c.playlist.id)}
        {@const full = c.adds.length === 0}
        <button class="group-row row" disabled={full} onclick={() => void add(c)}>
          <span class="art"><Cover src={c.playlist.cover} /></span>
          <span class="words text"><span class="clip">{c.playlist.name}</span><span class="sub">{W.playlistHolds(c.holds, p.songIds.length, c.playlist.songCount)}</span></span>
          {#if full}<Icon name="check" />{/if}
        </button>
      {/each}
    </div>
  {/if}
  <div class="end"><button type="button" class="text-action" onclick={close}>{W.cancel}</button></div>
</dialog>

<style>
  .card {
    max-height: min(600px, calc(100dvh - 64px));
  }
  .field {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    padding: 0 14px;
    border-radius: 20px;
    border: var(--hairline) solid var(--hair);
    background: var(--page);
  }
  .field:focus-within {
    border-color: var(--accent);
  }
  .field :global(.icon) {
    width: 18px;
    height: 18px;
    color: var(--ink2);
  }
  input {
    all: unset;
    flex: 1;
    min-width: 0;
    font-size: 15px;
  }
  .list {
    overflow-y: auto;
    min-height: 0;
    flex-shrink: 1;
  }
  /* It has them all: in ink, the check saying so, only nothing to press. */
  .row:disabled {
    opacity: 1;
  }
  /* The hairline starts where the name does, past the cover. */
  .list .row::after {
    left: 64px;
  }
  .row > :global(.icon:last-child:not(:first-child)) {
    color: var(--ink);
    margin-left: 0;
  }
  .art {
    width: 36px;
    flex: none;
  }
</style>
