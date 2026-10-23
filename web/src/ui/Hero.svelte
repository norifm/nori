<script lang="ts">
  // An album's, playlist's or artist's head, as Apple Music has it: the cover, the title, who and what,
  // Play and Shuffle, and the page's own buttons.
  import type { Snippet } from "svelte";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  let {
    cover,
    title,
    round = false,
    fallback = "compact-disc",
    by,
    facts = "",
    onplay,
    actions,
  }: { cover?: string; title: string; round?: boolean; fallback?: string; by?: Snippet; facts?: string; onplay: (shuffle: boolean) => void; actions?: Snippet } = $props();
</script>

<div class="hero" class:circle={round}>
  <div class="art"><Cover src={cover} {round} {fallback} /></div>
  <div class="text">
    <h1>{title}</h1>
    {#if by}<div class="by">{@render by()}</div>{/if}
    {#if facts}<div class="facts">{facts}</div>{/if}
    <div class="hero-actions">
      <button class="pill solid" onclick={() => onplay(false)}><Icon name="play-solid" />{W.play}</button>
      <button class="pill" onclick={() => onplay(true)}><Icon name="shuffle" />{W.shuffle}</button>
      <span class="grow"></span>
      {@render actions?.()}
    </div>
  </div>
</div>

<style>
  .hero {
    display: flex;
    align-items: flex-end;
    gap: 32px;
    margin-bottom: 28px;
  }
  .art {
    width: 240px;
    flex: none;
  }
  .art :global(.cover) {
    box-shadow: var(--shadow);
  }
  .circle .art :global(.cover) {
    border-radius: 50%;
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
    font-weight: 700;
    letter-spacing: -0.3px;
    line-height: 1.15;
  }
  .by {
    font-size: 20px;
    color: var(--ink2);
  }
  .by :global(a:hover) {
    color: var(--ink);
  }
  .facts {
    font-size: 12px;
    font-weight: 600;
    color: var(--ink2);
    text-transform: uppercase;
    letter-spacing: 0.02em;
  }
  .hero-actions {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 18px;
  }
  .hero-actions .pill {
    min-width: 108px;
  }
  .grow {
    flex: 1;
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
      font-size: 28px;
    }
    .by {
      font-size: 17px;
    }
    .hero-actions {
      justify-content: center;
      flex-wrap: wrap;
    }
    .hero-actions .pill {
      flex: 1;
      max-width: 170px;
    }
    .grow {
      display: none;
    }
  }
</style>
