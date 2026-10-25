<script lang="ts">
  // The player bar on a wide screen, as the desktop's player pill: one outline along the bottom. The song
  // on the left (its cover opens the full player, a right click its menu); shuffle, previous, play, next
  // and repeat with the seek bar under them in the middle; where it plays, the seal, the lyrics and the
  // queue (each opening the side sheet, lit while it shows) and the volume on the right.
  import { player } from "../state/player.svelte";
  import { songMenu } from "../state/menus";
  import { href } from "../state/route.svelte";
  import { pointer, ui, type Pane } from "../state/ui.svelte";
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Controls from "./Controls.svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import OutputButton, { kindGlyph } from "./OutputButton.svelte";
  import Seal from "./Seal.svelte";
  import Seek from "./Seek.svelte";
  import Volume from "./Volume.svelte";

  const s = $derived(player.song);
  const n = $derived(player.now);
  const panes: [Pane, string, string][] = [
    ["lyrics", "emoji-sing-right-note", W.lyrics],
    ["queue", "playlist", W.queue],
  ];
  /** The pane showing: the full player's while it is open, else the side sheet's. */
  const showing = $derived(ui.full ? ui.pane : ui.side);
</script>

<footer class="bar chrome" id="bar" aria-label={W.nowPlayingBar}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="song" oncontextmenu={(e) => s && (e.preventDefault(), void songMenu(s, pointer(e), { inPlayer: true }))}>
    {#if s}
      <button class="art" aria-label={s.title} onclick={() => (ui.full = true)}><Cover src={s.cover} radius="14px" /></button>
      <div class="text">
        <span class="title clip">{s.title}</span>
        {#if n?.device}
          <span class="sub there clip"><Icon name={kindGlyph[n.deviceKind ?? ""] ?? "computer"} />{W.playingOn(n.device)}</span>
        {:else}
          <span class="sub clip">
            {#if s.artistId}<a href={href("artist", s.artistId)}>{s.artist}</a>{:else}{s.artist}{/if}{#if s.albumId}&nbsp;·&nbsp;<a href={href("album", s.albumId)}>{s.album}</a>{/if}{#if n?.mixing}&nbsp;·&nbsp;{W.autoMix}{/if}
          </span>
        {/if}
        {#if player.held}<button class="held" onclick={() => player.unhold()}>{W.tapToPlay}</button>{/if}
      </div>
    {:else}
      <div class="art"><Cover radius="14px" /></div>
      <span class="title">{W.nothingPlaying}</span>
    {/if}
  </div>
  <div class="middle">
    <Controls />
    {#if s}<Seek inline times={!ui.full} />{/if}
  </div>
  <div class="right">
    {#if !n?.asks && remote.devices.on}<span class="output-slot" class:on={showing === "devices"}><OutputButton /></span>{/if}
    {#if s && !n?.asks}<Seal what="song" id={s.id} on={!!s.starred} />{/if}
    {#each panes as [pane, icon, label] (pane)}
      <button class="glyph quiet" class:on={showing === pane} title={label} aria-label={label} aria-pressed={showing === pane} data-pane={pane} onclick={() => ui.show(pane)}><Icon name={icon} /></button>
    {/each}
    <Volume mute />
  </div>
</footer>

<style>
  .bar {
    position: fixed;
    left: 50%;
    transform: translateX(-50%);
    bottom: 12px;
    width: min(1040px, calc(100% - 32px));
    height: 64px;
    z-index: 30;
    display: grid;
    /* Equal sides, so the transport sits in the middle whatever the song is called. */
    grid-template-columns: minmax(0, 1fr) clamp(260px, 34%, 420px) minmax(0, 1fr);
    align-items: center;
    gap: 12px;
    /* The cover sits 6px in; the corner is its 14px corner plus that. */
    padding: 0 12px 0 6px;
    border-radius: 20px;
    border: 1px solid var(--outline);
  }
  .song {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .art {
    width: 52px;
    flex: none;
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .title {
    font-size: 13px;
    font-weight: 600;
    line-height: 18px;
  }
  .sub {
    color: var(--ink2);
    font-size: 12px;
    line-height: 16px;
  }
  .sub a:hover {
    color: var(--ink);
  }
  .there {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--ink);
  }
  .there :global(.icon) {
    width: 14px;
    height: 14px;
    flex: none;
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
  }
  .middle :global(.time) {
    font-size: 11px;
    min-width: 34px;
    text-align: center;
  }
  .middle :global(.track) {
    height: 3px;
  }
  .middle :global(.run::after) {
    width: 8px;
    height: 8px;
    right: -4px;
    top: -2.5px;
  }
  .right {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
    min-width: 0;
  }
  .right :global(.glyph),
  .right :global(.output) {
    width: 34px;
    min-width: 34px;
    height: 34px;
  }
  /* Every line glyph here one size (the seal's too), so their lines match. */
  .right :global(.icon),
  .right :global(.output .icon) {
    width: 20px;
    height: 20px;
  }
  .right :global(.seal:not(.on)) {
    color: var(--ink2);
  }
  .output-slot.on :global(.output) {
    color: var(--accent);
  }
  .right :global(.vol) {
    margin-left: 4px;
  }
  @media (max-width: 899px) {
    .right :global(.vol) {
      display: none;
    }
  }
</style>
