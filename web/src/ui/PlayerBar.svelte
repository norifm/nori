<script lang="ts">
  // The player bar on a wide screen: one outline along the bottom. The song on the left (its cover opens
  // the full player); shuffle, previous, play, next and repeat with the seek bar under them in the middle;
  // the seal, where it plays (the device's glyph, lit while another one plays), lyrics, the queue and the
  // volume on the right.
  import { player } from "../state/player.svelte";
  import { href } from "../state/route.svelte";
  import { ui, type Pane } from "../state/ui.svelte";
  import { W } from "../words";
  import Controls from "./Controls.svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import OutputButton from "./OutputButton.svelte";
  import Seal from "./Seal.svelte";
  import Seek from "./Seek.svelte";
  import Volume from "./Volume.svelte";

  const s = $derived(player.song);
  const n = $derived(player.now);
  const panes: [Pane, string, string][] = [
    ["lyrics", "emoji-sing-right-note", W.lyrics],
    ["queue", "playlist", W.queue],
  ];
</script>

<footer class="bar chrome" id="bar" aria-label={W.nowPlayingBar}>
  <div class="song">
    {#if s}
      <button class="art" aria-label={s.title} onclick={() => (ui.full = true)}><Cover src={s.cover} radius="16px" /></button>
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
      <div class="art"><Cover radius="16px" /></div>
      <span class="sub">{W.nothingPlaying}</span>
    {/if}
  </div>
  <div class="middle">
    <Controls />
    {#if s}<Seek inline times={!ui.full} />{/if}
  </div>
  <div class="right">
    {#if s && !n?.asks}<Seal what="song" id={s.id} on={!!s.starred} />{/if}
    {#if !n?.asks}<OutputButton />{/if}
    {#each panes as [pane, icon, label] (pane)}
      <button class="glyph quiet" class:on={ui.full && ui.pane === pane} title={label} aria-label={label} data-pane={pane} onclick={() => ui.show(pane)}><Icon name={icon} /></button>
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
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .title {
    font-size: 15px;
    line-height: 20px;
  }
  .sub {
    color: color-mix(in srgb, var(--ink) 65%, transparent);
    font-size: 13px;
  }
  .sub a:hover {
    color: var(--ink);
  }
  .held {
    font-size: 12px;
    font-weight: 600;
    text-align: left;
    color: var(--accent);
  }
  .middle {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 0;
  }
  .right {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    min-width: 0;
  }
  .right :global(.glyph),
  .right :global(.output) {
    width: 40px;
    min-width: 40px;
    height: 40px;
  }
  /* Every line glyph here one size (the seal's too), so their lines match. */
  .right :global(.icon),
  .right :global(.output .icon) {
    width: 21px;
    height: 21px;
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
