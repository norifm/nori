<script lang="ts">
  // The full player: the cover, the song, the seek bar and controls, AutoMix, speed and pitch; beside
  // them on a wide screen (in the cover's place on a phone) the lyrics, the queue, other devices or the
  // jam.
  import { player } from "../state/player.svelte";
  import { songMenu } from "../state/menus";
  import { href } from "../state/route.svelte";
  import { ui, type Pane } from "../state/ui.svelte";
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Controls from "./Controls.svelte";
  import Cover from "./Cover.svelte";
  import Heart from "./Heart.svelte";
  import Icon from "./Icon.svelte";
  import Seek from "./Seek.svelte";
  import SoundRow from "./SoundRow.svelte";
  import PaneView from "./PaneView.svelte";

  const s = $derived(player.song!);
  const n = $derived(player.now);
  /** On a wide screen something is always beside the cover: the lyrics unless another was picked. */
  const pane = $derived<Pane | null>(ui.pane ?? (ui.wide ? "lyrics" : null));
  const panes: [Pane, string, string][] = [
    ["lyrics", "lyrics", W.lyrics],
    ["devices", "devices", W.playOn],
    ["queue", "queue", W.queue],
    ["jam", "jam", W.jam],
  ];
  const close = () => {
    ui.full = false;
    ui.pane = null;
  };
</script>

<div class="full" class:wide={ui.wide} class:paned={!!pane} role="dialog" aria-modal="true" aria-label={s.title} id="full">
  <button class="round close" aria-label={W.close} title={W.close} onclick={close}><Icon name="down-wide" /></button>
  <section class="main">
    {#if !pane || ui.wide}
      <div class="art" class:held={!player.playing}><Cover src={s.cover} /></div>
    {:else}
      <div class="small-head">
        <div class="small-art"><Cover src={s.cover} /></div>
        <div class="clip"><div class="clip title-s">{s.title}</div><div class="sub clip">{s.artist}</div></div>
      </div>
      <div class="pane-phone"><PaneView {pane} /></div>
    {/if}
    <div class="title-row">
      <div class="names">
        <span class="title clip">{s.title}</span>
        {#if s.artistId}<a class="artist clip" href={href("artist", s.artistId)} onclick={close}>{s.artist}</a>{:else}<span class="artist clip">{s.artist}</span>{/if}
      </div>
      {#if !n?.jam}<Heart what="song" id={s.id} on={!!s.starred} />{/if}
      <button class="round" aria-label={W.more} title={W.more} onclick={() => void songMenu(s, [], true)}><Icon name="more" /></button>
    </div>
    <Seek />
    <div class="badges">
      {#if n?.device}<span class="badge"><Icon name="devices" />{W.playingOn(n.device)}</span>{/if}
      {#if n?.mixing}<span class="badge">{W.autoMix}</span>{/if}
      {#if n?.jam}<span class="badge">{W.jam}</span>{/if}
      {#if player.held}<button class="badge" onclick={() => player.unhold()}>{W.tapToPlay}</button>{/if}
    </div>
    <Controls big />
    {#if !n?.device && !n?.jam}<SoundRow />{/if}
    <nav class="pane-tabs">
      {#each panes as [p, icon, label] (p)}
        {#if p !== "devices" || remote.devices.on}
          <button class="round tab" class:on={pane === p} aria-pressed={pane === p} aria-label={label} title={label} data-pane={p} onclick={() => (ui.pane = ui.pane === p && !ui.wide ? null : p)}><Icon name={icon} /></button>
        {/if}
      {/each}
    </nav>
  </section>
  {#if ui.wide && pane}
    <section class="side"><PaneView {pane} /></section>
  {/if}
</div>

<style>
  .full {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: var(--panel);
    display: flex;
    justify-content: center;
    padding: 56px var(--gutter) calc(16px + env(safe-area-inset-bottom));
    overflow: hidden;
    animation: rise 0.28s cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes rise {
    from {
      transform: translateY(24px);
      opacity: 0;
    }
  }
  .close {
    position: absolute;
    top: 12px;
    left: 12px;
    color: var(--dim);
  }
  .close :global(.icon) {
    width: 26px;
    height: 26px;
  }
  .main {
    width: 100%;
    max-width: 420px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-height: 0;
  }
  .art {
    width: 100%;
    max-width: min(420px, 52vh);
    margin: auto auto 8px;
    transition: transform 0.35s cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .art :global(.cover) {
    border-radius: 10px;
    box-shadow: var(--shadow);
  }
  .art.held {
    transform: scale(0.86);
  }
  .small-head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .small-art {
    width: 56px;
    flex: none;
  }
  .title-s {
    font-weight: 600;
  }
  .pane-phone {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0 calc(-1 * var(--gutter));
    padding: 0 var(--gutter);
  }
  .paned:not(.wide) .title-row {
    display: none;
  }
  .title-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .names {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title {
    font: 700 21px/1.2 var(--display);
  }
  .artist {
    font-size: 18px;
    color: var(--accent);
  }
  .badges {
    display: flex;
    gap: 10px;
    justify-content: center;
    min-height: 16px;
    margin: -6px 0 -4px;
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
  }
  .badge :global(.icon) {
    width: 14px;
    height: 14px;
  }
  .pane-tabs {
    display: flex;
    justify-content: space-around;
    margin-top: auto;
    padding-top: 6px;
  }
  .tab {
    color: var(--dim);
  }
  .tab.on {
    color: var(--accent);
    background: var(--chosen);
  }
  /* Wide: the cover and controls on the left, the pane on the right. */
  .wide {
    gap: 64px;
    padding: 56px 48px 32px;
  }
  .wide .main {
    justify-content: center;
  }
  .wide .art {
    margin: 0 auto 8px;
  }
  .side {
    flex: 1;
    max-width: 560px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .wide .pane-tabs {
    margin-top: 6px;
    justify-content: center;
    gap: 24px;
  }
</style>
