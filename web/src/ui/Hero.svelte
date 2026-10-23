<script lang="ts">
  // An album's, playlist's or artist's head: the cover, the title, who, what the list holds ("2026 · 4 songs ·
  // 8:00 · MP3 128 kbps"), Play and Shuffle, and the page's own buttons (the seal, what the core offers for
  // its songs, a menu). The page and the chrome wear the season's paper dyed with the cover while it is open
  // (not in Midnight).
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
    round = false,
    fallback = "compact-disc",
    by,
    label = "",
    onplay,
    actions,
  }: { cover?: string; title: string; round?: boolean; fallback?: string; by?: Snippet; label?: string; onplay: (shuffle: boolean) => void; actions?: Snippet } = $props();

  // The cover's colors, lent to the page and the chrome while it is open; the season's until they come.
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

<div class="hero" class:circle={round}>
  <div class="art"><Cover src={cover} {round} radius="var(--r-card)" {fallback} /></div>
  <div class="text">
    <h1>{title}</h1>
    {#if by}<div class="by">{@render by()}</div>{/if}
    {#if label}<div class="label">{label}</div>{/if}
    <div class="hero-actions">
      <button class="pill" onclick={() => onplay(false)}><Icon name="play-solid" />{W.play}</button>
      <button class="pill quiet" onclick={() => onplay(true)}><Icon name="shuffle" />{W.shuffle}</button>
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
