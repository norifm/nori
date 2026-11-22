<script lang="ts">
  // The first run's questions once its first profile is in, over the app opening under them, one to a page
  // (the core's `setup_steps`). A question shows what the answer chosen does on four covers, its
  // answers as rows: a pick is taken at once and plays across the preview, Continue moves on. The look is
  // Settings' own, taken as it is picked, and Start listening ends the run.
  import { Tween } from "svelte/motion";
  import { cubicOut } from "svelte/easing";
  import { ask } from "../rpc";
  import { W } from "../words";
  import Icon from "./Icon.svelte";
  import Look from "./Look.svelte";
  import { arrive, still } from "./motion";

  type Step = { step: string; answers: string[] };
  let { steps, end }: { steps: Step[]; end: () => void } = $props();

  let at = $state(0);
  /** Which way the last move went: 1 forward, -1 back. */
  let way = $state(1);
  /** The answers given so far, by step, so going back shows them. */
  let given = $state<Record<string, string>>({});
  const shown = $derived(steps[at]);
  const last = $derived(at === steps.length - 1);
  const chosen = $derived(given[shown.step]);

  function go(by: number) {
    way = by;
    at += by;
  }

  /** Picks played so far: each one runs the playhead once across the preview. */
  let plays = $state(0);

  async function pick(a: string) {
    if (chosen === a) return;
    given[shown.step] = a;
    plays++;
    await ask("setup_answer", a);
  }

  /** The covers the preview plays (`setup_covers`), the same on every client; plates until they come. */
  let played = $state<(string | null)[]>(Array(4).fill(null));
  void ask<Uint8Array<ArrayBuffer>[]>("setup_covers").then((covers) => (played = covers.map((c) => URL.createObjectURL(new Blob([c], { type: "image/webp" })))));

  /** Where each of `style`'s songs starts after the last, as a share of a cover's side, and how much of each
   * cover fades in over the one before: albums apart with a cut between; songs that flow overlapping,
   * blended; nonstop drawn close, blended longer. */
  const spacing = (style: string | undefined): [number, number] => (style === "AutoMix" ? [0.72, 0.28] : style === "AutoMixDj" ? [0.5, 0.5] : [1.12, 0]);
  const motion = () => ({ duration: still() ? 0 : 320, easing: cubicOut });
  const stepBy = new Tween(spacing(undefined)[0]);
  const fade = new Tween(0);
  const beat = new Tween(0);
  $effect(() => {
    if (shown.step !== "Listening") return;
    const [s, f] = spacing(chosen);
    void stepBy.set(s, motion());
    void fade.set(f, motion());
    void beat.set(chosen === "AutoMixDj" ? 1 : 0, motion());
  });

  /** The preview's width; a cover's side fits the widest arrangement. */
  let width = $state(0);
  const side = $derived(width / (1 + spacing(undefined)[0] * (played.length - 1)));
  const span = (by: number) => side * (1 + by * (played.length - 1));
  const start = (by: number) => (width - span(by)) / 2;
  /** The DJ's beat: ticks across the covers. */
  const BEATS = 17;
</script>

{#snippet listening()}
  <div class="covers" bind:clientWidth={width} style:height="{side + 24}px">
    {#each played as src, i (src ?? i)}
      <span
        class="cover"
        style:left="{start(stepBy.current) + i * stepBy.current * side}px"
        style:width="{side}px"
        style:mask-image={i > 0 && fade.current > 0 ? `linear-gradient(to right, transparent, #000 ${fade.current * 100}%)` : null}
      >
        {#if src}<img {src} alt="" />{/if}
      </span>
    {/each}
    {#if beat.current > 0}
      <span class="beat" style:left="{start(stepBy.current)}px" style:width="{span(stepBy.current)}px" style:top="{side + 12}px" style:opacity={beat.current}>
        {#each Array.from({ length: BEATS }, (_, i) => i) as i (i)}<span class="tick"></span>{/each}
      </span>
    {/if}
    {#if plays && !still()}
      {#key plays}
        {@const [s] = spacing(chosen)}
        <span class="playhead" style:--from="{start(s)}px" style:--to="{start(s) + span(s)}px" style:height="{side + 12}px"></span>
      {/key}
    {/if}
  </div>
{/snippet}

<div id="setup" class="setup">
  <div class="bar">
    <span class="side">{#if at > 0}<button class="glyph" id="setup-back" aria-label={W.back} onclick={() => go(-1)}><Icon name="nav-arrow-left" /></button>{/if}</span>
    <span class="dots">{#each steps as s, i (s.step)}<span class="dot" class:on={i === at}></span>{/each}</span>
    <span class="side end">{#if !last}<button class="text-action" id="setup-skip" onclick={() => go(1)}>{W.setupSkip}</button>{/if}</span>
  </div>
  {#key at}
    <div class="page" in:arrive={{ way }}>
      <h1>{W.setupSteps[shown.step] ?? shown.step}</h1>
      {#if shown.step === "Listening"}
        <div class="preview" id="setup-preview">{@render listening()}</div>
        <div class="group answers">
          {#each shown.answers as a (a)}
            {@const [name, sub] = W.setupAnswers[a] ?? [a, ""]}
            <button class="group-row" data-answer={a} aria-pressed={chosen === a} onclick={() => void pick(a)}>
              <span class="words"><span>{name}</span><span class="sub">{sub}</span></span>
              {#if chosen === a}<span class="check"><Icon name="check" /></span>{/if}
            </button>
          {/each}
        </div>
      {:else}
        <Look />
      {/if}
    </div>
  {/key}
  <div class="start">
    {#if last}<button class="pill" id="setup-start" onclick={end}>{W.setupStart}</button>
    {:else}<button class="pill" id="setup-continue" disabled={!chosen} onclick={() => go(1)}>{W.continueOn}</button>{/if}
  </div>
</div>

<style>
  .setup {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    flex-direction: column;
    background: var(--page);
    color: var(--ink);
    padding: env(safe-area-inset-top) 0 env(safe-area-inset-bottom);
  }
  .bar {
    display: flex;
    align-items: center;
    height: 52px;
    padding: 0 4px;
    flex: none;
  }
  .side {
    width: 96px;
    display: flex;
  }
  .side.end {
    justify-content: flex-end;
    padding-right: 4px;
  }
  .dots {
    flex: 1;
    display: flex;
    justify-content: center;
    gap: 6px;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--ink) 22%, transparent);
  }
  .dot.on {
    width: 18px;
    background: var(--accent);
  }
  .page {
    flex: 1;
    overflow-y: auto;
    width: 100%;
    max-width: 560px;
    margin: 0 auto;
    padding: 20px var(--gutter) 24px;
  }
  h1 {
    font-size: 31px;
    line-height: 38px;
  }
  .preview {
    margin-top: 22px;
    padding: 22px 18px 14px;
    border-radius: var(--r-tile);
    border: var(--hairline) solid var(--hair);
    background: var(--surface);
  }
  .covers {
    position: relative;
  }
  .cover {
    position: absolute;
    top: 0;
    aspect-ratio: 1;
    overflow: hidden;
    border-radius: var(--r-cover);
    background: var(--plate);
  }
  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .beat {
    position: absolute;
    height: 8px;
    display: flex;
    justify-content: space-between;
  }
  .tick {
    width: 1.5px;
    background: color-mix(in srgb, var(--ink) 32%, transparent);
  }
  /* A pick playing: a line in the accent once across the covers, from above them to below. */
  .playhead {
    position: absolute;
    top: -6px;
    width: 2px;
    margin-left: -1px;
    background: var(--accent);
    animation: play 900ms linear forwards;
  }
  @keyframes play {
    from {
      left: var(--from);
    }
    to {
      left: var(--to);
      visibility: hidden;
    }
  }
  .answers {
    margin-top: 14px;
  }
  .answers .group-row {
    min-height: 64px;
  }
  .check {
    color: var(--accent);
  }
  .check :global(.icon) {
    width: 20px;
    height: 20px;
  }
  .page :global(.look) {
    margin-top: 6px;
  }
  .start {
    flex: none;
    width: 100%;
    max-width: 560px;
    margin: 0 auto;
    padding: 12px var(--gutter) 16px;
  }
  .start .pill {
    width: 100%;
  }
</style>
