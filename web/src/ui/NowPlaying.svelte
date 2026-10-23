<script lang="ts">
  // The full player: the cover, the song, the seek bar and controls; beside them on a wide screen (in
  // the cover's place on a phone) the lyrics, the queue (with the jam over it) or other devices, in a panel
  // with its tabs. It wears the season's paper dyed with the cover, the cover blurred behind it (not in
  // Midnight).
  import { player } from "../state/player.svelte";
  import { songMenu } from "../state/menus";
  import { href } from "../state/route.svelte";
  import { looks } from "../state/theme.svelte";
  import { beside, ui, type Pane } from "../state/ui.svelte";
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Controls from "./Controls.svelte";
  import Cover from "./Cover.svelte";
  import Seal from "./Seal.svelte";
  import Icon from "./Icon.svelte";
  import OutputButton from "./OutputButton.svelte";
  import Seek from "./Seek.svelte";
  import PaneView from "./PaneView.svelte";
  import Volume from "./Volume.svelte";
  import { coverLook, type CoverLook } from "../state/coverLook";

  const s = $derived(player.song!);
  const n = $derived(player.now);
  // `s` is new on every refresh; the cover's address compares by value.
  const cover = $derived(s.cover);
  /** On a wide screen something is always beside the cover: the lyrics unless another was picked. */
  const pane = $derived<Pane | null>(ui.pane ?? (ui.wide ? "lyrics" : null));
  const panes: [Pane, string, string][] = [
    ["lyrics", "emoji-sing-right-note", W.lyrics],
    ["devices", "computer", W.playOn],
    ["queue", "playlist", W.queue],
  ];
  const close = () => {
    ui.full = false;
    ui.pane = null;
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
  <button class="glyph close" aria-label={W.close} title={W.close} onclick={close}><Icon name="nav-arrow-down" /></button>
  <section class="main">
    {#if !pane || ui.wide}
      <div class="art" class:held={!player.playing}><Cover src={s.cover} radius="var(--r-card)" /></div>
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
    {#if ui.wide}<div class="volume"><Volume /></div>{/if}
    {#if !ui.wide}
      <nav class="pane-tabs">
        {#each panes as [p, icon, label] (p)}
          {#if p !== "devices" || remote.devices.on}
            <button class="glyph quiet" class:on={pane === p} aria-pressed={pane === p} aria-label={label} title={label} data-pane={p} onclick={() => (ui.pane = ui.pane === p && !ui.wide ? null : p)}><Icon name={icon} /></button>
          {/if}
        {/each}
      </nav>
    {/if}
  </section>
  {#if ui.wide && pane}
    <section class="side">
      <nav class="head" aria-label={W.more}>
        {#each panes as [p, icon, label] (p)}
          {#if p !== "devices" || remote.devices.on}
            <button class="tab-btn" class:on={pane === p} aria-pressed={pane === p} data-pane={p} onclick={() => (ui.pane = p)}><Icon name={icon} /><span>{label}</span></button>
          {/if}
        {/each}
      </nav>
      <div class="body"><PaneView {pane} /></div>
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
  /* The cover blurred, behind everything; drawn small and stretched, as it is already soft. */
  .wash {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }
  .main,
  .side {
    position: relative;
  }
  .close {
    position: absolute;
    top: 8px;
    left: 8px;
    color: var(--ink2);
    z-index: 1;
  }
  .close :global(.icon) {
    width: 24px;
    height: 24px;
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
    gap: 2px;
    margin-right: -8px;
  }
  .names {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title {
    font: 700 24px/1.25 var(--display);
    letter-spacing: -0.3px;
  }
  .artist {
    font-size: 17px;
    font-weight: 500;
    color: color-mix(in srgb, var(--ink) 60%, transparent);
  }
  .title-row :global(.seal) {
    color: var(--ink);
  }
  .title-row :global(.seal.on) {
    color: var(--accent);
  }
  /* The seal and the menu, one size, so their lines match. */
  .title-row :global(.icon) {
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
  .badge {
    font-size: 12px;
    font-weight: 600;
    color: var(--ink2);
  }
  .badge.held {
    color: var(--accent);
  }
  .pane-tabs {
    display: flex;
    justify-content: space-around;
    margin-top: auto;
    padding-top: 6px;
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
    max-width: 620px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--veil);
    border: var(--hairline) solid var(--hair);
    border-radius: var(--r-sheet);
    overflow: hidden;
  }
  .volume {
    display: flex;
    justify-content: center;
  }
  .volume :global(.vol) {
    width: 200px;
  }
  .head {
    display: flex;
    gap: 4px;
    padding: 12px;
    border-bottom: var(--hairline) solid var(--hair);
  }
  .tab-btn {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 34px;
    padding: 0 14px;
    border-radius: 999px;
    border: var(--hairline) solid transparent;
    color: var(--ink2);
    font-size: 14px;
    font-weight: 500;
  }
  .tab-btn :global(.icon) {
    width: 20px;
    height: 20px;
  }
  .tab-btn.on {
    color: var(--accent);
    background: var(--surface);
    border-color: var(--hair);
  }
  /* A column, so the lyrics scroll in themselves (keeping the sung line in view) under their soft edges. */
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    padding: 14px 22px;
    scrollbar-width: none;
  }
</style>
