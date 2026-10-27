<script lang="ts">
  // A mix on Home's "For you", as the desktop's and Android's: a portrait card on the season's paper dyed
  // with the mix (the core's `mix_card_colours`), its covers a square on top (four as a mosaic) running
  // into that paper, its name in Literata and a line under it in the card's ink. A right click opens its
  // menu.
  import { wholeMenu } from "../state/menus";
  import { href } from "../state/route.svelte";
  import type { MixTile } from "../state/types";
  import { pointer } from "../state/ui.svelte";
  import { W } from "../words";

  let { mix }: { mix: MixTile } = $props();
  const title = $derived(W.mixNames[mix.name] ?? mix.name);
  const line = $derived(W.mixCaption(mix.favourites));
  const covers = $derived(mix.covers.length >= 4 ? mix.covers.slice(0, 4) : mix.covers.slice(0, 1));
</script>

<a
  class="mix"
  href={href("mix", mix.id)}
  style:--card={mix.page}
  style:--card-ink={mix.ink}
  oncontextmenu={(e) => (e.preventDefault(), wholeMenu("mix", mix.id, { title, line, cover: mix.covers[0] }, pointer(e)))}
>
  <span class="art" class:mosaic={covers.length === 4}>
    {#each covers as c (c)}<img src={c} alt="" loading="lazy" decoding="async" />{/each}
  </span>
  <span class="words">
    <span class="title clip">{title}</span>
    <span class="line clip">{line}</span>
  </span>
</a>

<style>
  .mix {
    position: relative;
    display: flex;
    flex-direction: column;
    aspect-ratio: 150 / 206;
    border-radius: 18px;
    overflow: hidden;
    background: var(--card);
    color: var(--card-ink);
    transition: background 240ms ease-out;
  }
  .art {
    position: relative;
    display: grid;
    aspect-ratio: 1;
    background: color-mix(in srgb, var(--card-ink) 10%, var(--card));
  }
  .art.mosaic {
    grid-template-columns: 1fr 1fr;
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  /* The covers rubbed out into the card's paper from under halfway down. */
  .art::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(transparent 45%, var(--card));
  }
  .words {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
    min-width: 0;
    padding: 0 12px;
  }
  .title {
    font: 600 18px/1.25 var(--display);
    letter-spacing: -0.3px;
  }
  .line {
    font-size: 12px;
    color: color-mix(in srgb, var(--card-ink) 70%, transparent);
  }
  .mix::after {
    content: "";
    position: absolute;
    inset: 0;
    background: transparent;
    transition: background 180ms ease-out;
    pointer-events: none;
  }
  .mix:hover::after {
    background: color-mix(in srgb, var(--card) 15%, transparent);
  }
</style>
