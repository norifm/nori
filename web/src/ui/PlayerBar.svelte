<script lang="ts">
  // The player on a wide screen: one outline along the bottom, the page blurred inside it. What plays on
  // the left (its cover opens the full player); previous, play and next with the seek bar under them in
  // the middle; where it plays, the seal, the lyrics, the queue and the volume on the right. The outline's
  // corners are the cover's, as far out again as the cover stands in.
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
      <button class="art" aria-label={s.title} onclick={() => (ui.full = true)}><Cover src={s.cover} radius="10px" /></button>
      <div class="text">
        <span class="title clip">{s.title}</span>
        <span class="line clip">
          {#if s.artistId}<a href={href("artist", s.artistId)}>{s.artist}</a>{:else}{s.artist}{/if}
          {#if s.albumId}&nbsp;·&nbsp;<a href={href("album", s.albumId)}>{s.album}</a>{/if}
        </span>
        {#if player.held}<button class="held" onclick={() => player.unhold()}>{W.tapToPlay}</button>{/if}
      </div>
    {:else}
      <div class="art"><Cover radius="10px" /></div>
      <span class="line">{W.nothingPlaying}</span>
    {/if}
  </div>
  <div class="middle">
    <div class="transport"><Controls /></div>
    {#if s}<Seek inline times={!ui.full} />{/if}
  </div>
  <div class="right">
    {#if !n?.asks}<OutputButton />{/if}
    {#if s && !n?.asks}<Seal what="song" id={s.id} on={!!s.starred} size={22} />{/if}
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
    height: 72px;
    z-index: 30;
    display: grid;
    /* Equal sides, so the transport sits in the middle whatever the song is called. */
    grid-template-columns: minmax(0, 1fr) minmax(320px, 1.1fr) minmax(0, 1fr);
    align-items: center;
    gap: 14px;
    padding: 0 12px 0 8px;
    border-radius: 18px;
  }
  .song {
    display: flex;
    align-items: center;
    gap: 11px;
    min-width: 0;
  }
  .art {
    width: 56px;
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
  .line {
    color: color-mix(in srgb, var(--ink) 65%, transparent);
    font-size: 13px;
    line-height: 17px;
  }
  .line a:hover {
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
    gap: 0;
  }
  .transport {
    display: flex;
    justify-content: center;
    gap: 6px;
    height: 44px;
    align-items: center;
  }
  .transport :global(.glyph) {
    width: 44px;
    height: 44px;
  }
  .right {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    min-width: 0;
  }
  .right :global(.glyph .icon) {
    width: 22px;
    height: 22px;
  }
  .right :global(.vol) {
    margin-left: 6px;
  }
  @media (max-width: 899px) {
    .bar {
      grid-template-columns: minmax(0, 1fr) minmax(280px, 1.2fr) auto;
    }
    .right :global(.vol) {
      display: none;
    }
  }
</style>
