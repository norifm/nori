<script lang="ts">
  // An album's, playlist's or artist's head: the cover, the title, who, what the list holds ("2026 · 4 songs ·
  // 8:00 · MP3 128 kbps"), Play and Shuffle (dimmed until the songs are read, and on a list with none,
  // `playable`), and the page's own buttons (the seal, what the core offers for its songs, a menu). A right
  // click on the cover or the title opens the menu of the page's `item` (its `itemMark`). The page and the
  // chrome wear the season's paper dyed with the cover while it is open,
  // and the cover's own wash runs across the page's top into it, scrolling with the head, as the desktop's
  // page (not in Midnight).
  import type { Snippet } from "svelte";
  import { coverLook } from "../state/coverLook";
  import type { itemMark } from "../state/menus";
  import { fade } from "svelte/transition";
  import { looks, RECOLOR_MS } from "../state/theme.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import { still } from "./motion";

  let {
    cover,
    title,
    round = false,
    fallback = "compact-disc",
    by,
    label = "",
    onplay,
    playable = true,
    actions,
    item,
  }: { cover?: string; title: string; round?: boolean; fallback?: string; by?: Snippet; label?: string; onplay: (shuffle: boolean) => void; playable?: boolean; actions?: Snippet; item?: ReturnType<typeof itemMark> } = $props();

  // The cover's colors, lent to the page and the chrome while it is open; the season's until they come.
  const me = {};
  let own = $state.raw<ImageData | null>(null);
  $effect(() => {
    void looks.rev;
    const src = cover;
    let gone = false;
    if (src)
      void coverLook(src).then((l) => {
        if (gone) return;
        ui.tint = l ? { by: me, vars: l.vars } : null;
        own = l?.own ?? null;
      });
    else {
      ui.tint = null;
      own = null;
    }
    return () => {
      gone = true;
    };
  });
  let washed = $state<HTMLCanvasElement>();
  $effect(() => {
    if (!washed || !own) return;
    washed.width = own.width;
    washed.height = own.height;
    washed.getContext("2d")?.putImageData(own, 0, 0);
  });
  $effect(() => () => {
    if (ui.tint?.by === me) ui.tint = null;
  });
</script>

<!-- Midnight wears none: it goes as the colors do. -->
{#if own}<div class="head-wash" aria-hidden="true" out:fade={{ duration: still() ? 0 : RECOLOR_MS }}><canvas bind:this={washed}></canvas></div>{/if}
<div class="hero" class:circle={round}>
  <div class="art" {...item}><Cover src={cover} {round} radius="var(--hero-art)" {fallback} /></div>
  <div class="text">
    <h1 {...item}>{title}</h1>
    {#if by}<div class="by">{@render by()}</div>{/if}
    {#if label && !ui.wide}<div class="label">{label}</div>{/if}
    <div class="hero-actions">
      <button class="pill" disabled={!playable} onclick={() => onplay(false)}><Icon name="play-solid" />{W.play}</button>
      <button class="pill quiet shuffle" disabled={!playable} aria-label={W.shuffle} title={W.shuffle} onclick={() => onplay(true)}><Icon name="shuffle" /><span>{W.shuffle}</span></button>
      <span class="grow"></span>
      {@render actions?.()}
    </div>
  </div>
</div>
<!-- On a wide window what the list holds stands on a line of its own over it, as the desktop's. -->
{#if ui.wide}<div class="info clip">{label}</div>{/if}

<style>
  /* Across the top of the page that scrolls (main), behind its head: drawn small and stretched, as it is
     already soft, under the page's paper growing from 15 % to whole. */
  .head-wash {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 440px;
    z-index: -1;
    pointer-events: none;
    animation: washed 400ms ease-out;
  }
  @keyframes washed {
    from {
      opacity: 0;
    }
  }
  .head-wash canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
  .head-wash::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(color-mix(in srgb, var(--page) 15%, transparent), color-mix(in srgb, var(--page) 55%, transparent) 55%, var(--page));
  }
  .hero {
    --hero-art: var(--r-card);
    display: flex;
    align-items: flex-end;
    gap: 32px;
    margin-bottom: 28px;
  }
  .art {
    width: 240px;
    flex: none;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  h1 {
    font-size: 34px;
    line-height: 1.15;
  }
  .by {
    font-size: 20px;
    font-weight: 500;
    color: var(--ink2);
  }
  .by :global(a:hover) {
    color: var(--ink);
  }
  .label {
    font-size: 13px;
    color: var(--ink2);
  }
  .hero-actions {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 18px;
  }
  .hero-actions .pill {
    min-width: 116px;
  }
  .grow {
    flex: 1;
  }
  /* The desktop's head on a wide window: the cover 270 px, the name 70 px down its side, the buttons at its
     foot: Shuffle a bare glyph before Play, as on Android. */
  @media (min-width: 720px) {
    .hero {
      --hero-art: 8px;
      align-items: stretch;
      gap: 28px;
      margin-bottom: 26px;
    }
    .art {
      width: 270px;
    }
    .text {
      gap: 2px;
      padding-top: 70px;
    }
    h1 {
      font-size: 30px;
      letter-spacing: -0.3px;
    }
    .by {
      color: var(--ink);
    }
    .hero-actions {
      gap: 6px;
      margin-top: auto;
      padding-top: 16px;
    }
    .hero-actions .pill {
      min-width: 180px;
      padding: 0 24px;
    }
    .hero-actions .shuffle {
      order: -1;
      min-width: 48px;
      width: 48px;
      height: 48px;
      padding: 0;
      background: none;
    }
    .shuffle :global(.icon) {
      width: 24px;
      height: 24px;
    }
    .shuffle span {
      display: none;
    }
    .hero-actions :global(.glyph .icon) {
      width: 24px;
      height: 24px;
    }
    .info {
      height: 49px;
      line-height: 48px;
      margin-bottom: 26px;
      border-bottom: 1px solid var(--hair);
      font-size: 13px;
      color: var(--ink2);
    }
  }
  @media (max-width: 719px) {
    .hero {
      flex-direction: column;
      align-items: center;
      text-align: center;
      gap: 18px;
    }
    .art {
      width: min(260px, 70vw);
    }
    h1 {
      font-size: 27px;
    }
    .by {
      font-size: 17px;
    }
    .hero-actions {
      align-self: stretch;
      justify-content: center;
      gap: 4px;
    }
    .hero-actions .pill {
      flex: 1;
      min-width: 0;
      max-width: 170px;
      padding: 0 12px;
    }
    .grow {
      display: none;
    }
  }
</style>
