<script lang="ts">
  // The player bar on a wide screen: one outline along the bottom. The song on the left (its cover opens
  // the full player); shuffle, previous, play, next and repeat with the seek bar under them in the middle;
  // the seal, the speaker (Play On), lyrics, the queue and the volume on the right.
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import { href } from "../state/route.svelte";
  import { ui, type Pane } from "../state/ui.svelte";
  import { W } from "../words";
  import Controls from "./Controls.svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import Seal from "./Seal.svelte";
  import Seek from "./Seek.svelte";
  import Volume from "./Volume.svelte";

  const s = $derived(player.song);
  const n = $derived(player.now);
  const panes = $derived<[Pane, string, string][]>([
    ["devices", n?.device ? "devices" : "speaker", W.playOn],
    ["lyrics", "lyrics", W.lyrics],
    ["queue", "queue", W.queue],
  ]);
</script>

<footer class="bar chrome" id="bar" aria-label={W.nothingPlaying}>
  <div class="song">
    {#if s}
      <button class="art" aria-label={s.title} onclick={() => (ui.full = true)}><Cover src={s.cover} /></button>
      <div class="text">
        <span class="title clip">{s.title}</span>
        <span class="sub clip">
          {#if n?.device}{W.playingOn(n.device)}{:else}
            {#if s.artistId}<a href={href("artist", s.artistId)}>{s.artist}</a>{:else}{s.artist}{/if}
            {#if s.albumId}&nbsp;·&nbsp;<a href={href("album", s.albumId)}>{s.album}</a>{/if}
          {/if}
          {#if n?.mixing}&nbsp;·&nbsp;{W.autoMix}{/if}
        </span>
        {#if player.held}<button class="held" onclick={() => player.unhold()}>{W.tapToPlay}</button>{/if}
      </div>
    {:else}
      <div class="art"><Cover /></div>
      <span class="sub">{W.nothingPlaying}</span>
    {/if}
  </div>
  <div class="middle">
    <Controls />
    {#if s}<Seek inline times={!ui.full} />{/if}
  </div>
  <div class="right">
    {#if s && !n?.jam}<Seal what="song" id={s.id} on={!!s.starred} />{/if}
    {#each panes as [pane, icon, label] (pane)}
      {#if pane !== "devices" || remote.devices.on}
        <button class="round" class:on={ui.full && ui.pane === pane} title={label} aria-label={label} data-pane={pane} onclick={() => ui.show(pane)}><Icon name={icon} /></button>
      {/if}
    {/each}
    <Volume />
  </div>
</footer>

<style>
  .bar {
    position: fixed;
    left: 50%;
    transform: translateX(-50%);
    bottom: 16px;
    width: min(1040px, calc(100% - 32px));
    height: 78px;
    z-index: 30;
    display: grid;
    /* Equal sides, so the transport sits in the middle whatever the song is called. */
    grid-template-columns: minmax(0, 1fr) minmax(360px, 1.1fr) minmax(0, 1fr);
    align-items: center;
    gap: 14px;
    /* The cover sits 7px in on three sides; the corner is its 16px corner plus that. */
    padding: 0 16px 0 7px;
    border-radius: 23px;
  }
  .song {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .art {
    width: 64px;
    flex: none;
  }
  .art :global(.cover) {
    border-radius: 16px;
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .title {
    font-size: 13.5px;
    font-weight: 600;
  }
  .sub {
    color: var(--ink2);
    font-size: 12px;
  }
  .sub a:hover {
    text-decoration: underline;
  }
  .held {
    font-size: 11px;
    font-weight: 600;
    text-align: left;
    color: var(--accent);
  }
  .middle {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
  }
  .right {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
    min-width: 0;
  }
  .right .round {
    color: var(--ink2);
  }
  .right .round.on {
    color: var(--accent);
  }
  .right :global(.vol) {
    margin-left: 8px;
  }
  @media (max-width: 899px) {
    .bar {
      grid-template-columns: minmax(0, 1fr) minmax(300px, 1.2fr) auto;
    }
    .right :global(.vol) {
      display: none;
    }
  }
</style>
