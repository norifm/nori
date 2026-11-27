<script lang="ts">
  // A loading page's placeholders, as the core sketches it (`sketch`): plates in the page's quiet ink
  // (`--sketch`) where its items will stand, each with the real one's shape and size: a head as Hero lays
  // it, shelves of cards as Shelf and Card, a grid as Cards', rows as SongList's. With nori's Motion setting
  // at Full a slow band of a lighter ink (`--sheen`) passes over each, on the compositor; otherwise they
  // stand still.
  import { ask } from "../rpc";
  import Shelf from "./Shelf.svelte";

  let { reading, known = false }: { reading: string; known?: boolean } = $props();

  type Part = { part: "Head" | "Shelf" | "Grid" | "Rows"; art: "Square" | "Round" | "Number" | "Bare" };
  let parts = $state.raw<Part[]>([]);
  $effect(() => {
    let gone = false;
    void ask<string>("sketch", reading, known).then((s) => {
      if (!gone) parts = JSON.parse(s) as Part[];
    });
    return () => {
      gone = true;
    };
  });
  /** Lines of their own lengths, so rows read as titles. */
  const SHARES = [62, 46, 74, 54, 68, 40, 58, 70];
  const share = (i: number) => SHARES[i % SHARES.length];
  /** Rows and grids run past the window's foot; the sketch is cut there. */
  const FILL = 14;
</script>

<div class="sketch" aria-hidden="true">
  {#each parts as p, n (n)}
    {#if p.part === "Head"}
      <div class="hero" class:circle={p.art === "Round"}>
        <div class="p art"></div>
        <div class="text">
          <span class="line h1"><i class="p" style:width="60%"></i></span>
          <span class="line by"><i class="p" style:width="34%"></i></span>
          <div class="actions"><i class="p pill"></i><i class="p pill"></i></div>
        </div>
      </div>
    {:else if p.part === "Shelf"}
      <section class="shelf">
        <div class="section-title"><span class="line"><i class="p" style:width="9em"></i></span></div>
        <Shelf count={8}>
          {#each { length: 8 } as _, i}{@render card(p.art, i)}{/each}
        </Shelf>
      </section>
    {:else if p.part === "Grid"}
      <div class="grid">{#each { length: FILL } as _, i}{@render card(p.art, i)}{/each}</div>
    {:else}
      <ol class="rows" class:numbered={p.art === "Number"}>
        {#each { length: FILL } as _, i}
          <li>
            {#if p.art === "Number"}<span class="n"><i class="p"></i></span>{:else if p.art !== "Bare"}<span class="p row-art" class:round={p.art === "Round"}></span>{/if}
            <span class="line title"><i class="p" style:width="{share(i)}%"></i></span>
            {#if p.art !== "Bare"}<span class="line artist"><i class="p" style:width="{share(i + 3) * 0.6}%"></i></span>{/if}
            {#if p.art === "Square"}<span class="line album"><i class="p" style:width="{share(i + 5) * 0.7}%"></i></span>{/if}
          </li>
        {/each}
      </ol>
    {/if}
  {/each}
</div>

{#snippet card(art: Part["art"], i: number)}
  <div class="card" class:round={art === "Round"}>
    <div class="p cover"></div>
    <span class="line title"><i class="p" style:width="{share(i) + 20}%"></i></span>
    <span class="line sub"><i class="p" style:width="{share(i + 3) * 0.7}%"></i></span>
  </div>
{/snippet}

<style>
  /* A plate, and the band crossing it: a transform alone, so it runs on the compositor. */
  .p {
    display: block;
    position: relative;
    overflow: hidden;
    background: var(--sketch);
  }
  .p::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(90deg, transparent, var(--sheen), transparent);
    transform: translateX(-100%);
    animation: sheen 1500ms linear infinite;
  }
  :global(.still) .p::after {
    display: none;
  }
  @keyframes sheen {
    to {
      transform: translateX(100%);
    }
  }
  /* A line of words: as tall as the line, its bar as tall as the capitals. */
  .line {
    display: flex;
    align-items: center;
    min-width: 0;
  }
  .line .p {
    height: 0.62em;
    border-radius: 999px;
  }
  .section-title .line {
    display: inline-flex;
    width: 100%;
  }

  .card {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .cover {
    aspect-ratio: 1;
    border-radius: var(--r-card);
  }
  .round .cover {
    border-radius: 50%;
  }
  .card .title {
    margin-top: 8px;
    font-size: 14px;
    height: 19px;
  }
  .card .sub {
    font-size: 12.5px;
    height: 16px;
  }
  .round .line {
    justify-content: center;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 24px var(--grid-gap);
  }

  .rows {
    list-style: none;
    margin: 0 -8px;
    padding: 0;
  }
  li {
    display: grid;
    grid-template-columns: 44px 1fr;
    grid-template-rows: 21px 17px;
    align-items: center;
    column-gap: 14px;
    row-gap: 1px;
    padding: 9px 8px;
    min-height: 62px;
    padding-right: 136px;
  }
  .row-art {
    grid-row: span 2;
    width: 44px;
    height: 44px;
    border-radius: var(--r-cover);
  }
  .row-art.round {
    border-radius: 50%;
  }
  .rows .title {
    font-size: 16px;
  }
  .rows .artist {
    font-size: 13px;
  }
  .album {
    display: none;
  }
  .numbered li {
    grid-template-columns: 30px 1fr;
    min-height: 57px;
  }
  .n {
    grid-row: span 2;
    display: flex;
    justify-content: center;
  }
  .n .p {
    width: 12px;
    height: 9px;
    border-radius: 999px;
  }

  .hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 18px;
    margin-bottom: 28px;
  }
  .art {
    width: min(260px, 70vw);
    aspect-ratio: 1;
    border-radius: var(--r-card);
  }
  .circle .art {
    border-radius: 50%;
  }
  .text {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    align-self: stretch;
  }
  .text .line {
    justify-content: center;
    width: 100%;
  }
  .h1 {
    font-size: 27px;
    height: 31px;
  }
  .by {
    font-size: 17px;
    height: 22px;
  }
  .actions {
    display: flex;
    justify-content: center;
    gap: 4px;
    margin-top: 18px;
    align-self: stretch;
  }
  .pill {
    flex: 1;
    max-width: 170px;
    height: 44px;
    border-radius: 999px;
  }

  @media (max-width: 719px) {
    .grid {
      grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
      gap: 18px var(--grid-gap);
    }
  }
  /* The desktop's frame on a wide window: rows 56 px (44 numbered) in columns, the head beside its cover. */
  @media (min-width: 720px) {
    .card {
      padding-bottom: 4px;
    }
    .card .title {
      height: 18px;
    }
    li {
      grid-template-columns: 44px 4fr 3fr 3fr;
      grid-template-rows: 44px;
      column-gap: 12px;
      min-height: 56px;
      padding: 6px 127px 6px 8px;
    }
    .row-art {
      grid-row: auto;
      border-radius: 5px;
    }
    .rows .title,
    .rows .artist,
    .album {
      font-size: 13px;
    }
    .album {
      display: flex;
    }
    .numbered li {
      grid-template-columns: 24px 2fr 1fr;
      min-height: 44px;
      padding-block: 0;
    }
    .n {
      grid-row: auto;
      justify-content: flex-end;
    }
    .hero {
      flex-direction: row;
      align-items: stretch;
      gap: 28px;
      margin-bottom: 26px;
    }
    .art {
      width: 270px;
      border-radius: 8px;
    }
    .text {
      align-items: flex-start;
      gap: 2px;
      padding-top: 70px;
    }
    .text .line {
      justify-content: flex-start;
    }
    .h1 {
      font-size: 30px;
      height: 35px;
    }
    .by {
      font-size: 20px;
      height: 25px;
    }
    .actions {
      justify-content: flex-start;
      margin-top: auto;
      padding-top: 16px;
      gap: 6px;
    }
    .pill {
      flex: none;
      width: 180px;
      height: 48px;
    }
    .pill:first-child {
      width: 48px;
      border-radius: 50%;
    }
  }
</style>
