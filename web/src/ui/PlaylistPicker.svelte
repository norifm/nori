<script lang="ts">
  // Adding songs to a playlist, as Spotify's picker: a field that finds a playlist as it is typed in, or
  // names a new one; the playlists under it, a click adding the songs.
  import { library } from "../state/library.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let dialog = $state<HTMLDialogElement>();
  const p = $derived(ui.picking!);
  let name = $state("");
  const typed = $derived(name.trim());
  const found = $derived(p.playlists.filter((x) => x.name.toLowerCase().includes(typed.toLowerCase())));
  const close = () => (ui.picking = null);
  $effect(() => dialog?.showModal());

  async function add(id: string, title: string) {
    const songIds = p.songIds;
    close();
    await library.change({ kind: "addToPlaylist", id, songIds });
    ui.say(W.addedTo(title));
  }
  async function create() {
    if (!typed) return;
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
    <input bind:value={name} placeholder={W.findOrName} aria-label={W.findOrName} autocomplete="off" />
  </form>
  <div class="list">
    <button class="row new" disabled={!typed} onclick={() => void create()}><span class="disc"><Icon name="plus" /></span><span class="clip">{typed ? W.newPlaylistNamed(typed) : W.newPlaylist}</span></button>
    {#each found as x (x.id)}
      <button class="row" onclick={() => void add(x.id, x.name)}><span class="disc"><Icon name="playlist" /></span><span class="clip">{x.name}</span></button>
    {/each}
  </div>
  <div class="end"><button type="button" class="pill small quiet" onclick={close}>{W.cancel}</button></div>
</dialog>

<style>
  .card {
    padding: 20px;
    border: var(--hairline) solid var(--hair);
    border-radius: var(--r-sheet);
    background: var(--surface);
    color: var(--ink);
    width: min(400px, calc(100vw - 32px));
    max-height: min(560px, calc(100vh - 64px));
  }
  .card[open] {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .card::backdrop {
    background: var(--scrim);
  }
  h2 {
    font-size: 20px;
    line-height: 26px;
  }
  .field {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 38px;
    padding: 0 12px;
    border-radius: 19px;
    border: 1px solid var(--hair);
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
    font-size: 14px;
  }
  .list {
    overflow-y: auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    margin: 0 -8px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 48px;
    padding: 0 8px;
    border-radius: 12px;
    text-align: left;
    font-size: 14px;
  }
  .row:hover:enabled {
    background: var(--hover);
  }
  .row:disabled {
    opacity: 1;
    cursor: default;
  }
  .disc {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    flex: none;
    border-radius: 8px;
    background: color-mix(in srgb, var(--ink) 7%, transparent);
  }
  .disc :global(.icon) {
    width: 18px;
    height: 18px;
  }
  .new {
    font-weight: 600;
  }
  .end {
    display: flex;
    justify-content: flex-end;
  }
</style>
