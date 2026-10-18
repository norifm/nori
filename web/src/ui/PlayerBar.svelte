<script lang="ts">
  // The player bar under the page on a wide screen: the song (its cover opens the full player), the
  // controls with the seek bar, and the panels beside the page (lyrics, the queue, other devices).
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import { href } from "../state/route.svelte";
  import { ui, type Pane } from "../state/ui.svelte";
  import { W } from "../words";
  import Controls from "./Controls.svelte";
  import Cover from "./Cover.svelte";
  import Heart from "./Heart.svelte";
  import Icon from "./Icon.svelte";
  import Seek from "./Seek.svelte";

  const s = $derived(player.song);
  const n = $derived(player.now);
  const panes: [Pane, string, string][] = [
    ["lyrics", "lyrics", W.lyrics],
    ["queue", "queue", W.queue],
    ["devices", "devices", W.playOn],
  ];
</script>

<footer class="bar" id="bar" aria-label={W.nothingPlaying}>
  <div class="song">
    {#if s}
      <button class="art" aria-label={s.title} onclick={() => (ui.full = true)}><Cover src={s.cover} /></button>
      <div class="text">
        <span class="title clip">{s.title}</span>
        <span class="sub clip">
          {#if s.artistId}<a href={href("artist", s.artistId)}>{s.artist}</a>{:else}{s.artist}{/if}
          {#if s.albumId}&nbsp;—&nbsp;<a href={href("album", s.albumId)}>{s.album}</a>{/if}
        </span>
      </div>
      {#if !n?.jam}<Heart what="song" id={s.id} on={!!s.starred} />{/if}
    {:else}
      <div class="art"><Cover /></div>
      <span class="sub">{W.nothingPlaying}</span>
    {/if}
  </div>
  <div class="middle">
    <Controls />
    <div class="seek-row">
      {#if s}<Seek />{/if}
    </div>
    {#if n?.device}<span class="on-device"><Icon name="devices" />{W.playingOn(n.device)}</span>{/if}
    {#if n?.mixing}<span class="badge">{W.autoMix}</span>{/if}
    {#if player.held}<button class="badge held" onclick={() => player.unhold()}>{W.tapToPlay}</button>{/if}
  </div>
  <div class="panes">
    {#each panes as [pane, icon, label] (pane)}
      {#if pane !== "devices" || remote.devices.on}
        <button class="round" class:on={ui.panel === pane} title={label} aria-label={label} aria-pressed={ui.panel === pane} data-pane={pane} onclick={() => ui.show(pane)}><Icon name={icon} /></button>
      {/if}
    {/each}
  </div>
</footer>

<style>
  .bar {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    height: var(--bar-h);
    z-index: 30;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(320px, 520px) minmax(0, 1fr);
    align-items: center;
    gap: 24px;
    padding: 0 16px;
    background: var(--bar);
    border-top: 1px solid var(--hair);
  }
  .song {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .art {
    width: 48px;
    flex: none;
  }
  .art :global(.cover) {
    border-radius: 5px;
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .title {
    font-size: 14px;
    font-weight: 500;
  }
  .sub a:hover {
    text-decoration: underline;
  }
  .middle {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 0;
    padding-top: 2px;
  }
  .seek-row {
    height: 26px;
    margin-top: -4px;
  }
  .seek-row :global(.times) {
    padding-top: 2px;
  }
  .on-device,
  .badge {
    position: absolute;
    right: 0;
    top: 10px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--accent);
  }
  .on-device {
    left: 0;
    right: auto;
  }
  .on-device :global(.icon) {
    width: 14px;
    height: 14px;
  }
  .held {
    top: auto;
    bottom: -2px;
  }
  .panes {
    display: flex;
    justify-content: flex-end;
    gap: 4px;
  }
  .panes .round {
    color: var(--dim);
  }
  .panes .round.on {
    color: var(--accent);
    background: var(--chosen);
  }
</style>
