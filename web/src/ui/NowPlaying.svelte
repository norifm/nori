<script lang="ts">
  // The full player, as Android's: the cover at the top, melting into the page dyed with it (the cover
  // blurred lies behind the title and the controls); the title with the seal and the menu, the seek bar,
  // then lyrics, previous, play, next and the queue, then where it plays. The lyrics or the queue take
  // the cover's place, dissolving into it. On a wide screen the cover (or the lyrics or the queue) is the
  // left half, going soft at its right edge, and the controls stand beside it. Opening, the cover grows
  // from the bar's thumbnail.
  import { fade } from "svelte/transition";
  import { coverLook, type CoverLook } from "../state/coverLook";
  import { songMenu } from "../state/menus";
  import { player } from "../state/player.svelte";
  import { href } from "../state/route.svelte";
  import { looks } from "../state/theme.svelte";
  import { ui, type Pane } from "../state/ui.svelte";
  import { W } from "../words";
  import Controls from "./Controls.svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import Lyrics from "./Lyrics.svelte";
  import OutputButton from "./OutputButton.svelte";
  import Queue from "./Queue.svelte";
  import Seal from "./Seal.svelte";
  import Seek from "./Seek.svelte";
  import Volume from "./Volume.svelte";

  const s = $derived(player.song!);
  const n = $derived(player.now);
  // `s` is new on every refresh; the cover's address compares by value.
  const cover = $derived(s.cover);
  const close = () => {
    ui.full = false;
    ui.pane = null;
  };
  const choose = (p: Pane) => (ui.pane = ui.pane === p ? null : p);

  // The cover's colors and wash; the page's own until they come (and in Midnight, which wears none).
  let look = $state<CoverLook | null>(null);
  $effect(() => {
    void looks.rev;
    const src = cover;
    let gone = false;
    if (src) void coverLook(src).then((l) => !gone && (look = l));
    else look = null;
    return () => (gone = true);
  });
  /** The wash: the blurred cover, its last row carried on below it, so it runs on under the controls. */
  let washed = $state<HTMLCanvasElement>();
  $effect(() => {
    const w = look?.wash;
    if (!washed || !w) return;
    const side = w.width;
    washed.width = side;
    washed.height = side * 2;
    const g = washed.getContext("2d")!;
    g.putImageData(w, 0, 0);
    g.drawImage(washed, 0, side - 1, side, 1, 0, side, side, side);
  });
  /** The share of the sleeve that melts, from the top of its band: the core's (`melt`). */
  const melt = $derived(look?.melt ?? 0.38);

  // The cover grows out of the bar's thumbnail as the player opens (unless the Motion setting asks for less).
  let sleeve = $state<HTMLElement>();
  $effect(() => {
    const el = sleeve;
    const from = document.querySelector("#mini .art, #bar .art")?.getBoundingClientRect();
    if (!el || !from || document.documentElement.classList.contains("still")) return;
    const to = el.getBoundingClientRect();
    if (!to.width) return;
    const k = from.width / to.width;
    el.animate(
      [
        { transform: `translate(${from.left - to.left}px, ${from.top - to.top}px) scale(${k})`, borderRadius: `${10 / k}px` },
        { transform: "none", borderRadius: "0px" },
      ],
      { duration: 380, easing: "cubic-bezier(0.05, 0.7, 0.1, 1)" },
    );
  });
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && !ui.sheet && !ui.devices && close()} />

<div class="full tinted" class:wide={ui.wide} class:paned={!!ui.pane} role="dialog" aria-modal="true" aria-label={s.title} id="full" style={look?.vars} style:--melt={melt}>
  {#if look?.wash}<canvas class="wash" bind:this={washed} aria-hidden="true"></canvas><div class="floor"></div>{/if}
  <section class="panel">
    {#key ui.pane}
      <div class="in" in:fade={{ duration: 360 }}>
        {#if !ui.pane}
          <div class="sleeve" bind:this={sleeve}><Cover src={s.cover} radius="0" /></div>
        {:else}
          {#if ui.pane === "lyrics"}<div class="head">
            <span class="thumb"><Cover src={s.cover} /></span>
            <span class="names"><span class="clip">{s.title}</span><span class="sub clip">{s.artist}</span></span>
            {#if !n?.asks}<Seal what="song" id={s.id} on={!!s.starred} size={22} />{/if}
            <button class="glyph" aria-label={W.more} title={W.more} onclick={() => void songMenu(s, [], true)}><Icon name="more-horiz" /></button>
          </div>{/if}
          <div class="pane" class:queue={ui.pane === "queue"}>{#if ui.pane === "lyrics"}<Lyrics />{:else}<Queue />{/if}</div>
        {/if}
      </div>
    {/key}
  </section>
  <section class="controls">
    {#if ui.pane !== "lyrics" || ui.wide}
      <div class="title-row">
        <div class="names">
          <span class="title clip">{s.title}</span>
          {#if s.artistId}<a class="artist clip" href={href("artist", s.artistId)} onclick={close}>{s.artist}</a>{:else}<span class="artist clip">{s.artist}</span>{/if}
          {#if s.album}{#if s.albumId}<a class="album clip" href={href("album", s.albumId)} onclick={close}>{s.album}</a>{:else}<span class="album clip">{s.album}</span>{/if}{/if}
        </div>
        {#if !n?.asks}<Seal what="song" id={s.id} on={!!s.starred} size={28} />{/if}
        <button class="glyph title-button" aria-label={W.more} title={W.more} onclick={() => void songMenu(s, [], true)}><Icon name="more-horiz" /></button>
      </div>
    {/if}
    <Seek />
    {#if player.held}<button class="held" onclick={() => player.unhold()}>{W.tapToPlay}</button>{/if}
    <div class="transport">
      <button class="glyph quiet" class:on={ui.pane === "lyrics"} aria-pressed={ui.pane === "lyrics"} aria-label={W.lyrics} title={W.lyrics} data-pane="lyrics" onclick={() => choose("lyrics")}><Icon name="emoji-sing-right-note" /></button>
      <Controls big />
      <button class="glyph quiet" class:on={ui.pane === "queue"} aria-pressed={ui.pane === "queue"} aria-label={W.queue} title={W.queue} data-pane="queue" onclick={() => choose("queue")}><Icon name="playlist" /></button>
    </div>
    {#if !n?.asks}<div class="output"><OutputButton named /></div>{/if}
    {#if ui.wide}<div class="volume"><Volume /></div>{/if}
  </section>
  <button class="glyph close" aria-label={W.close} title={W.close} onclick={close}><Icon name="nav-arrow-down" /></button>
</div>

<style>
  .full {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: var(--page);
    color: var(--ink);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    padding-bottom: env(safe-area-inset-bottom);
  }
  /* The blurred cover, the size of the sharp one and under it, carried on downwards; it calms into the
     page towards the bottom. */
  .wash {
    position: absolute;
    top: 0;
    left: 50%;
    translate: -50% 0;
    width: var(--side);
    height: calc(var(--side) * 2);
    pointer-events: none;
  }
  .floor {
    position: absolute;
    left: 0;
    right: 0;
    top: var(--side);
    bottom: 0;
    background: linear-gradient(transparent, color-mix(in srgb, var(--page) 22%, transparent) 45%, color-mix(in srgb, var(--page) 75%, transparent) 80%, var(--page));
    pointer-events: none;
  }
  .full {
    --side: min(100vw, calc(100dvh - 330px));
  }
  .panel {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .in {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  /* The sharp cover, its last rows rubbed out over the blurred one. */
  .sleeve {
    width: var(--side);
    margin: 0 auto;
    transform-origin: 0 0;
    overflow: hidden;
    -webkit-mask-image: linear-gradient(#000 calc((1 - var(--melt)) * 100%), rgb(0 0 0 / 0.58) calc((1 - var(--melt) / 2) * 100%), transparent);
    mask-image: linear-gradient(#000 calc((1 - var(--melt)) * 100%), rgb(0 0 0 / 0.58) calc((1 - var(--melt) / 2) * 100%), transparent);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 56px 8px 8px var(--gutter);
  }
  .thumb {
    width: 56px;
    flex: none;
  }
  .head .names {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    font-weight: 600;
  }
  .pane {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 0 var(--gutter);
  }
  .pane.queue {
    padding: 56px 26px 0;
  }
  .controls {
    position: relative;
    display: flex;
    flex-direction: column;
    padding: 0 0 12px;
  }
  .title-row {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 16px 0 26px;
  }
  .names {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title {
    font: 700 26px/34px var(--display);
    letter-spacing: -0.3px;
  }
  .artist {
    font-size: 16px;
    line-height: 22px;
    font-weight: 500;
    color: color-mix(in srgb, var(--ink) 60%, transparent);
  }
  .album {
    font-size: 14px;
    line-height: 19px;
    color: color-mix(in srgb, var(--ink) 45%, transparent);
  }
  .controls :global(.seal),
  .title-button {
    width: 44px;
    height: 44px;
    color: var(--ink);
  }
  .controls :global(.seal.on) {
    color: var(--accent);
  }
  .title-button :global(.icon) {
    width: 28px;
    height: 28px;
  }
  .controls > :global(.seek) {
    padding: 10px 26px 0;
  }
  .held {
    align-self: center;
    margin-top: 4px;
    font-size: 13px;
    font-weight: 600;
    color: var(--accent);
  }
  .transport {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 18px 18px 0;
  }
  .transport > .glyph :global(.icon) {
    width: 24px;
    height: 24px;
  }
  .output {
    display: flex;
    justify-content: center;
    padding: 18px var(--gutter) 0;
  }
  .output :global(.output.named) {
    color: var(--ink2);
  }
  .output :global(.output.lit) {
    color: var(--ink);
  }
  .volume {
    display: flex;
    justify-content: center;
    padding-top: 8px;
  }
  .volume :global(.vol) {
    width: 220px;
  }
  .close {
    position: absolute;
    top: 8px;
    left: 8px;
    background: var(--glass);
    -webkit-backdrop-filter: blur(12px);
    backdrop-filter: blur(12px);
  }
  .close :global(.icon) {
    width: 22px;
    height: 22px;
  }
  /* Wide: the cover is the left half, as tall as the window, going soft over its right 30 %; the lyrics or
     the queue take its place; the controls stand in the middle of the rest. */
  .wide {
    --side: min(100dvh, 50vw);
    flex-direction: row;
  }
  .wide .wash {
    top: 0;
    left: 0;
    translate: none;
    width: 100%;
    height: 100%;
  }
  .wide .floor {
    top: 0;
    background: color-mix(in srgb, var(--page) 35%, transparent);
  }
  .wide .panel {
    flex: none;
    width: var(--side);
  }
  .wide .sleeve {
    width: calc(var(--side) + 28px);
    height: 100dvh;
    margin: 0;
    -webkit-mask-image: linear-gradient(to right, #000 70%, transparent);
    mask-image: linear-gradient(to right, #000 70%, transparent);
  }
  .wide .sleeve :global(.cover) {
    height: 100%;
    aspect-ratio: auto;
  }
  .wide .controls {
    flex: 1;
    justify-content: center;
    max-width: 520px;
    margin: 0 auto;
    padding: 0 12px;
  }
  .wide .head {
    padding-top: 64px;
  }
  .wide .pane.queue {
    padding-top: 64px;
  }
</style>
