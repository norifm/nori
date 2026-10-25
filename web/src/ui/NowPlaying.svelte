<script lang="ts">
  // The full player. On a wide screen, as the desktop's: the cover, the song, the seek bar, the controls
  // and the volume in the middle of the left half; in the right half, with nothing round it, the devices,
  // the lyrics or the queue (with the jam over it) under their tabs, one always showing. On a phone the
  // cover, the song and the controls, the pane in the cover's place. It wears the season's paper dyed with
  // the cover, the cover blurred behind it (not in Midnight). A right click on the cover or the song opens
  // the song's menu.
  import { player } from "../state/player.svelte";
  import { songMenu } from "../state/menus";
  import { href } from "../state/route.svelte";
  import { looks } from "../state/theme.svelte";
  import { beside, pointer, ui, type Pane } from "../state/ui.svelte";
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Controls from "./Controls.svelte";
  import Cover from "./Cover.svelte";
  import Seal from "./Seal.svelte";
  import Icon from "./Icon.svelte";
  import OutputButton, { kindGlyph } from "./OutputButton.svelte";
  import Seek from "./Seek.svelte";
  import PaneView from "./PaneView.svelte";
  import PanelTabs from "./PanelTabs.svelte";
  import Volume from "./Volume.svelte";
  import { coverLook, type CoverLook } from "../state/coverLook";

  const s = $derived(player.song!);
  const n = $derived(player.now);
  // `s` is new on every refresh; the cover's address compares by value.
  const cover = $derived(s.cover);
  /** On a wide screen something is always beside the cover: the lyrics unless another was picked. */
  const pane = $derived<Pane | null>(ui.pane ?? (ui.wide ? "lyrics" : null));
  const close = () => {
    ui.full = false;
    ui.pane = null;
  };
  const menu = (e: MouseEvent) => {
    e.preventDefault();
    void songMenu(s, pointer(e), { inPlayer: true });
  };

  // The cover's colors and wash; the page's own until they come.
  let look = $state<CoverLook | null>(null);
  $effect(() => {
    void looks.rev;
    const src = cover;
    let gone = false;
    if (src) void coverLook(src).then((l) => !gone && (look = l));
    else look = null;
    return () => (gone = true);
  });
  let washed = $state<HTMLCanvasElement>();
  $effect(() => {
    const w = look?.wash;
    if (!washed || !w) return;
    washed.width = w.width;
    washed.height = w.height;
    washed.getContext("2d")?.putImageData(w, 0, 0);
  });
</script>

<div class="full tinted" class:wide={ui.wide} class:paned={!!pane} role="dialog" aria-modal="true" aria-label={s.title} id="full" style={look?.vars}>
  {#if look?.wash}<canvas class="wash" bind:this={washed} aria-hidden="true"></canvas>{/if}
  <button class="close" aria-label={W.close} title={W.close} onclick={close}><Icon name="nav-arrow-down" /></button>
  {#if ui.wide}
    <section class="left">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="art" class:held={!player.playing} oncontextmenu={menu}><Cover src={s.cover} radius="var(--r-cover)" /></div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="title-row" oncontextmenu={menu}>
        <span class="title clip">{s.title}</span>
        {#if !n?.asks}<Seal what="song" id={s.id} on={!!s.starred} />{/if}
      </div>
      <div class="line clip">
        {#if s.artistId}<a href={href("artist", s.artistId)} onclick={close}>{s.artist}</a>{:else}{s.artist}{/if}{#if s.album}<span class="album">&nbsp;— {#if s.albumId}<a href={href("album", s.albumId)} onclick={close}>{s.album}</a>{:else}{s.album}{/if}</span>{/if}
      </div>
      {#if n?.device}
        <button class="there" onclick={() => (ui.pane = "devices")}><Icon name={kindGlyph[n.deviceKind ?? ""] ?? "computer"} />{W.playingOn(n.device)}</button>
      {/if}
      {#if player.held}<button class="badge held" onclick={() => player.unhold()}>{W.tapToPlay}</button>{/if}
      <div class="seek"><Seek mixing={n?.mixing ? W.autoMix : ""} /></div>
      <div class="transport"><Controls big /></div>
      <div class="volume"><Volume ends /></div>
    </section>
    <div class="tabs"><PanelTabs current={pane} pick={(p) => (ui.pane = p)} /></div>
    <section class="side {pane}">{#if pane}<PaneView {pane} look="full" />{/if}</section>
  {:else}
    <section class="main">
      {#if !pane}
        <div class="art" class:held={!player.playing}><Cover src={s.cover} radius="var(--r-card)" /></div>
      {:else}
        <div class="small-head">
          <div class="small-art"><Cover src={s.cover} /></div>
          <div class="clip"><div class="clip title-s">{s.title}</div><div class="sub clip">{s.artist}</div></div>
        </div>
        <div class="pane-phone"><PaneView {pane} /></div>
      {/if}
      <div class="phone-title">
        <div class="names">
          <span class="name clip">{s.title}</span>
          {#if s.artistId}<a class="artist clip" href={href("artist", s.artistId)} onclick={close}>{s.artist}</a>{:else}<span class="artist clip">{s.artist}</span>{/if}
        </div>
        {#if !n?.asks}<Seal what="song" id={s.id} on={!!s.starred} />{/if}
        <button class="glyph" aria-label={W.more} title={W.more} onclick={(e) => void songMenu(s, beside(e), { inPlayer: true })}><Icon name="more-horiz" /></button>
      </div>
      <Seek />
      <div class="badges">
        {#if !n?.asks}<OutputButton named />{/if}
        {#if n?.mixing}<span class="badge">{W.autoMix}</span>{/if}
        {#if n?.jam}<span class="badge">{W.jam}</span>{/if}
        {#if player.held}<button class="badge held" onclick={() => player.unhold()}>{W.tapToPlay}</button>{/if}
      </div>
      <Controls big />
      <nav class="pane-tabs">
        {#each [["lyrics", "emoji-sing-right-note", W.lyrics], ["devices", "computer", W.devices], ["queue", "playlist", W.queue]] as [p, icon, label] (p)}
          {#if p !== "devices" || remote.devices.on}
            <button class="glyph quiet" class:on={pane === p} aria-pressed={pane === p} aria-label={label} title={label} data-pane={p} onclick={() => (ui.pane = ui.pane === p ? null : (p as Pane))}><Icon name={icon} /></button>
          {/if}
        {/each}
      </nav>
    </section>
  {/if}
</div>

<style>
  .full {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: var(--page);
    color: var(--ink);
    overflow: hidden;
    animation: shown 300ms ease-out;
  }
  @keyframes shown {
    from {
      opacity: 0;
    }
  }
  /* The cover blurred, behind everything; drawn small and stretched, as it is already soft. */
  .wash {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }
  .close {
    position: absolute;
    top: 8px;
    left: 8px;
    z-index: 1;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--page) 60%, transparent);
    border: 1px solid var(--outline);
    color: var(--ink);
  }
  .close :global(.icon) {
    width: 20px;
    height: 20px;
  }
  .clip {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Wide: the cover and the controls centred in the left half, as tall as they are. */
  .left {
    --side: min(460px, 100vh - 330px, 50vw - 120px);
    position: absolute;
    left: calc(25vw - var(--side) / 2);
    top: 50%;
    width: var(--side);
    transform: translateY(-50%);
    display: flex;
    flex-direction: column;
    animation: rise 420ms cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  @keyframes rise {
    from {
      translate: 0 30px;
    }
  }
  .wide .art {
    width: 100%;
    margin-bottom: 26px;
    cursor: default;
  }
  .art {
    transition: transform 500ms cubic-bezier(0.3, 1.35, 0.6, 1);
  }
  .art.held {
    transform: scale(0.9);
  }
  .title-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .title {
    flex: 1;
    min-width: 0;
    font-size: 16px;
    font-weight: 600;
    line-height: 22px;
  }
  .title-row :global(.seal) {
    width: 36px;
    height: 36px;
    min-width: 36px;
    margin: -4px -4px -4px 0;
    color: var(--ink);
  }
  .title-row :global(.seal .icon) {
    width: 28px;
    height: 28px;
  }
  .title-row :global(.seal.on) {
    color: var(--accent);
  }
  .line {
    font-size: 14px;
    line-height: 20px;
    color: color-mix(in srgb, var(--ink) 60%, transparent);
  }
  .album {
    color: color-mix(in srgb, var(--ink) 45%, transparent);
  }
  .line a:hover {
    color: var(--ink);
  }
  .there {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--ink);
    align-self: flex-start;
  }
  .there :global(.icon) {
    width: 15px;
    height: 15px;
  }
  .badge {
    font-size: 12px;
    font-weight: 600;
    color: var(--ink2);
  }
  .badge.held {
    color: var(--accent);
    align-self: flex-start;
    margin-top: 6px;
  }
  .seek {
    margin-top: 20px;
  }
  .seek :global(.track) {
    height: 5px;
    border-radius: 3px;
  }
  .seek :global(.times) {
    padding-top: 4px;
    font-size: 10px;
  }
  .transport {
    margin-top: 16px;
  }
  .transport :global(.controls.big) {
    justify-content: space-between;
    gap: 0;
  }
  .volume {
    margin-top: 14px;
  }
  .volume :global(.vol) {
    width: 100%;
  }
  /* The right half: the tabs, then what they show, with nothing round it. */
  .tabs {
    position: absolute;
    left: calc(50vw + 10px);
    top: 8px;
  }
  .side {
    position: absolute;
    left: calc(50vw + 10px);
    right: 60px;
    top: 64px;
    bottom: 32px;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .side.lyrics {
    left: calc(50vw + 20px);
    right: 120px;
    top: 48px;
    bottom: 56px;
  }
  .side.queue {
    top: 84px;
    bottom: 70px;
  }

  /* A phone: the cover, the song, the controls; the pane in the cover's place. */
  .full:not(.wide) {
    display: flex;
    justify-content: center;
    padding: 56px var(--gutter) calc(16px + env(safe-area-inset-bottom));
  }
  .main {
    position: relative;
    width: 100%;
    max-width: 420px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-height: 0;
  }
  .main .art {
    width: 100%;
    max-width: min(420px, 52vh);
    margin: auto auto 8px;
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
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    margin: 0 calc(-1 * var(--gutter));
    padding: 0 var(--gutter);
  }
  .paned .phone-title {
    display: none;
  }
  .phone-title {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-right: -8px;
  }
  .names {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .name {
    font: 700 24px/1.25 var(--display);
    letter-spacing: -0.3px;
  }
  .artist {
    font-size: 17px;
    font-weight: 500;
    color: color-mix(in srgb, var(--ink) 60%, transparent);
  }
  .phone-title :global(.seal) {
    color: var(--ink);
  }
  .phone-title :global(.seal.on) {
    color: var(--accent);
  }
  .phone-title :global(.icon) {
    width: 26px;
    height: 26px;
  }
  .badges {
    display: flex;
    gap: 10px;
    justify-content: center;
    align-items: center;
    min-height: 32px;
    margin: -6px 0 -4px;
  }
  .badges :global(.output.named) {
    height: 32px;
    font-size: 14px;
  }
  .badges :global(.output .icon) {
    width: 20px;
    height: 20px;
  }
  .pane-tabs {
    display: flex;
    justify-content: space-around;
    margin-top: auto;
    padding-top: 6px;
  }
</style>
