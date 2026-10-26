<script lang="ts">
  // An artist page's head, as the desktop's (after iOS's artist pages): the artist's picture across the
  // page's whole width, on the cover's own wash, dissolving into the page's paper; it drifts up slower than
  // the page as the page scrolls and fades as it goes. Over its foot, centered, the name and Play, Shuffle
  // and the page's own buttons. The page and the chrome wear the picture's colors while it is open.
  import type { Snippet } from "svelte";
  import { lendCover } from "../state/lend.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { name, cover, picture, onplay, actions }: { name: string; cover?: string; picture?: string; onplay: (shuffle: boolean) => void; actions?: Snippet } = $props();

  const lent = lendCover(() => cover);
  let loaded = $state(false);
  $effect(() => {
    void picture;
    loaded = false;
  });

  function paint(canvas: HTMLCanvasElement, w: ImageData) {
    canvas.width = w.width;
    canvas.height = w.height;
    canvas.getContext("2d")?.putImageData(w, 0, 0);
  }

  // How far the page has scrolled: the picture keeps 55 % of it back, and fades to a fifth over its height.
  let banner = $state<HTMLElement>();
  $effect(() => {
    const page = document.getElementById("main");
    if (!page || !banner) return;
    const b = banner;
    const moved = () => {
      const top = page.scrollTop;
      b.style.transform = `translateY(${(top * 0.55).toFixed(1)}px)`;
      b.style.opacity = String(1 + Math.max(-0.8, -top / b.offsetHeight));
    };
    moved();
    page.addEventListener("scroll", moved, { passive: true });
    return () => page.removeEventListener("scroll", moved);
  });
</script>

<div class="banner" bind:this={banner} aria-hidden="true">
  {#if lent.look?.own}{#key lent.look.own}<canvas class="wash" use:paint={lent.look.own}></canvas>{/key}{/if}
  {#if picture}<img src={picture} alt="" class:loaded onload={() => (loaded = true)} />{/if}
</div>
<header class="head">
  <h1>{name}</h1>
  <div class="actions">
    <button class="pill" onclick={() => onplay(false)}><Icon name="play-solid" />{W.play}</button>
    <button class="pill quiet" onclick={() => onplay(true)}><Icon name="shuffle" />{W.shuffle}</button>
    {@render actions?.()}
  </div>
</header>

<style>
  /* Across the top of the page that scrolls (main), behind the head. */
  .banner {
    --tall: max(420px, 64vh);
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: var(--tall);
    z-index: -1;
    pointer-events: none;
    overflow: hidden;
    will-change: transform;
  }
  .wash,
  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  .wash {
    animation: shown 400ms ease-out;
  }
  img {
    object-fit: cover;
    opacity: 0;
    transition: opacity 400ms ease-out;
  }
  img.loaded {
    opacity: 1;
  }
  @keyframes shown {
    from {
      opacity: 0;
    }
  }
  /* Clear over the top half, then the page's paper. */
  .banner::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(transparent 45%, color-mix(in srgb, var(--page) 70%, transparent) 78%, var(--page));
  }
  .head {
    min-height: calc(max(420px, 64vh) - var(--top-h) - 26px);
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    align-items: center;
    gap: 16px;
    padding-bottom: 8px;
    text-align: center;
  }
  h1 {
    font-family: var(--display);
    font-size: 44px;
    font-weight: 800;
    letter-spacing: -1px;
    line-height: 1.1;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .actions .pill {
    min-width: 116px;
  }
  @media (max-width: 719px) {
    h1 {
      font-size: 34px;
    }
    .banner {
      --tall: max(340px, 52vh);
    }
    .head {
      min-height: calc(max(340px, 52vh) - var(--top-h) - 26px);
    }
  }
</style>
