<script lang="ts">
  // An album's, artist's or playlist's page, as Android's HeroPage: the cover runs to the edges and melts
  // into the page over its last rows, the page and the chrome wear the season's paper dyed with it (not in
  // Midnight), the title rises into the melt with the artist under it, then Shuffle, the Play pill in the
  // page's ink and the page's seal, then the list's head: what it holds and what can be done with it. On a
  // wide screen the cover is the left half, going soft at its right edge, and the rest scrolls beside it.
  import type { Snippet } from "svelte";
  import { coverLook } from "../state/coverLook";
  import { looks } from "../state/theme.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  let {
    cover,
    title,
    big = false,
    fallback = "music-note",
    by,
    onplay,
    actions,
    label = "",
    tools,
    error,
    ready = true,
    children,
  }: {
    cover?: string;
    title: string;
    /** An artist's name, set larger. */
    big?: boolean;
    fallback?: string;
    by?: Snippet;
    onplay?: (shuffle: boolean) => void;
    actions?: Snippet;
    label?: string;
    tools?: Snippet;
    error?: string;
    ready?: boolean;
    children: Snippet;
  } = $props();

  // The cover's colors, lent to the page and the chrome while this page is open; the season's until they come.
  const me = {};
  $effect(() => {
    void looks.rev;
    const src = cover;
    let gone = false;
    if (src) void coverLook(src).then((l) => !gone && (ui.tint = l ? { by: me, vars: l.vars } : null));
    else ui.tint = null;
    return () => {
      gone = true;
    };
  });
  $effect(() => () => {
    if (ui.tint?.by === me) ui.tint = null;
  });
</script>

<div class="hero-page" class:wide={ui.wide}>
  <div class="sleeve">
    <Cover src={cover} radius="0" {fallback} />
    <div class="dissolve"></div>
  </div>
  <div class="side">
    <div class="words" class:lone={!by}>
      <h1 class:big>{title}</h1>
      {#if by}<div class="by">{@render by()}</div>{/if}
    </div>
    {#if onplay}
      <div class="buttons">
        <button class="glyph" aria-label={W.shuffle} title={W.shuffle} disabled={!ready} onclick={() => onplay(true)}><Icon name="shuffle" /></button>
        <button class="pill play" disabled={!ready} onclick={() => onplay(false)}><Icon name="play-solid" />{W.play}</button>
        {@render actions?.()}
      </div>
    {:else if actions}
      <div class="buttons lone">{@render actions()}</div>
    {/if}
    {#if label || tools}
      <div class="list-head"><span class="clip">{label}</span>{@render tools?.()}</div>
    {/if}
    {#if error}<p class="empty">{W.failed(error)}</p>
    {:else if ready}{@render children()}{/if}
  </div>
</div>

<style>
  .hero-page {
    padding-bottom: calc(var(--bar-h) + var(--gap-section));
  }
  /* The cover a little taller than square, cropped at its sides, its last rows melting into the page. */
  .sleeve {
    position: relative;
    height: calc(100vw + 44px);
    max-height: 120vh;
    overflow: hidden;
  }
  .sleeve :global(.cover) {
    width: 100%;
    height: 100%;
    aspect-ratio: auto;
  }
  .dissolve {
    position: absolute;
    inset: 0;
    background: var(--dissolve, linear-gradient(to bottom, transparent 60%, var(--page)));
  }
  .side {
    padding: 0 var(--gutter);
  }
  /* The title rises 34 px into the melt, and the melt's extra 44; with no artist line it sits that much lower. */
  .words {
    position: relative;
    margin-top: -78px;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
  }
  .words.lone {
    margin-top: -50px;
  }
  h1 {
    font-size: 27px;
    line-height: 34px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  h1.big {
    font-size: 32px;
    line-height: 40px;
  }
  .by {
    margin-top: 3px;
    font-size: 17px;
    line-height: 22px;
    font-weight: 500;
  }
  .by :global(a:hover) {
    text-decoration: underline;
  }
  .buttons {
    display: flex;
    align-items: center;
    gap: 4px;
    margin: 12px calc(var(--gap-tight) - var(--gutter)) 0;
  }
  .buttons.lone {
    justify-content: center;
    margin-top: 2px;
  }
  .play {
    flex: 1;
  }
  .buttons :global(.glyph:disabled),
  .play:disabled {
    opacity: 1;
  }
  .list-head {
    position: relative;
    display: flex;
    align-items: center;
    min-height: 48px;
    margin: 6px calc(var(--gap-tight) - var(--gutter)) 10px 0;
    font-size: 13px;
    color: var(--ink2);
  }
  .list-head > span {
    flex: 1;
  }
  .list-head::after {
    content: "";
    position: absolute;
    left: 0;
    right: calc(var(--gutter) - var(--gap-tight));
    bottom: 0;
    height: 1px;
    background: var(--line);
  }
  /* Wide: the cover is the left half, as tall as the window, going soft over its right 30 %; the words,
     the buttons and the list scroll beside it. */
  .wide {
    display: grid;
    grid-template-columns: clamp(360px, 45vw, 50vw) minmax(0, 1fr);
  }
  .wide .sleeve {
    position: sticky;
    top: 0;
    height: 100dvh;
    max-height: none;
    width: calc(100% + 28px);
    -webkit-mask-image: linear-gradient(to right, #000 70%, transparent);
    mask-image: linear-gradient(to right, #000 70%, transparent);
  }
  .wide .dissolve {
    display: none;
  }
  .wide .side {
    position: relative;
    padding: var(--top-h) 32px 0 var(--gutter);
  }
  .wide .words,
  .wide .words.lone {
    margin-top: 12px;
  }
</style>
