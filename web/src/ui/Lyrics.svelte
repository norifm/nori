<script lang="ts">
  // The lyrics, as the desktop draws them: the sung line lit (and in it the words sung so far, where the
  // lyrics time their words) by the core's lyric clock, asked again only when the clock says the next
  // change comes; nothing runs while the song holds or the page is hidden. The lines glide to keep the
  // sung one in place, by a transform the browser draws; a wheel or a drag browses them until the next
  // line. In the full player the lines away from the sung one go soft. A click on a line plays from it.
  import { player } from "../state/player.svelte";
  import { W } from "../words";

  /** `full`: the full player's, beside the cover; `side`: the side sheet's; `phone`: in the cover's place. */
  let { look = "phone" }: { look?: "full" | "side" | "phone" } = $props();

  let box = $state<HTMLDivElement>();
  let height = $state(0);
  const shown = $derived(player.shown && player.shown.song === player.song?.id ? player.shown : null);
  const synced = $derived(!!shown?.lyrics.synced);
  /** Lines play from where they are sung on a click, unless a jam guest asks the host for all it plays. */
  const taps = $derived(synced && !player.now?.asks);
  /** Where the sung line rests, as a share of the height. */
  const rest = $derived(look === "full" ? 0.36 : look === "side" ? 1 / 3 : 0.38);

  $effect(() => {
    void player.song?.id;
    player.wantLyrics();
  });

  $effect(() => {
    const a = player.anchor;
    if (!shown || !synced) return;
    let gone = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let frame: number | undefined;
    const step = async () => {
      timer = frame = undefined;
      const wait = await player.lyricsStep(16);
      if (gone || a.pace === 0 || wait <= 0 || document.hidden) return;
      if (wait <= 20) frame = requestAnimationFrame(() => void step());
      else timer = setTimeout(() => void step(), wait / a.pace);
    };
    const seen = () => {
      if (!document.hidden && timer === undefined && frame === undefined) void step();
    };
    void step();
    document.addEventListener("visibilitychange", seen);
    return () => {
      gone = true;
      clearTimeout(timer);
      if (frame !== undefined) cancelAnimationFrame(frame);
      document.removeEventListener("visibilitychange", seen);
    };
  });

  /** The line the lines are moved to: the one sung, or the last once all are over. */
  const aimed = $derived(shown ? Math.min(player.line, shown.lyrics.lines.length - 1) : -1);
  /** Each line's top and height, measured once laid out and again when a resize wraps them. */
  let tops = $state<{ top: number; height: number }[]>([]);
  /** Moved by hand from where the sung line puts them; null while they follow it. */
  let browsed = $state<number | null>(null);
  $effect(() => {
    void aimed;
    browsed = null;
  });
  const measure = () => {
    if (!box) return;
    tops = [...box.querySelectorAll<HTMLElement>("[data-i]")].map((e) => ({ top: e.offsetTop, height: e.offsetHeight }));
  };
  $effect(() => {
    void shown;
    if (!box) return;
    measure();
    const seen = new ResizeObserver(measure);
    seen.observe(box);
    return () => seen.disconnect();
  });
  const aimedTop = $derived(aimed >= 0 && tops[aimed] ? tops[aimed].top : 0);
  const offset = $derived(browsed ?? aimedTop - height * rest);

  function wheel(e: WheelEvent) {
    e.preventDefault();
    const last = tops.at(-1);
    browsed = Math.max(-height * rest, Math.min((last?.top ?? 0) - height * rest, offset + e.deltaY));
  }
  let dragFrom: { y: number; at: number } | null = null;

  /** How soft line `i` is: by how far it stands from the sung one, none while they are browsed. */
  function blur(i: number): number {
    if (look !== "full" || browsed !== null || aimed < 0 || !synced || !tops[i] || !tops[aimed]) return 0;
    const a = tops[aimed];
    const t = tops[i];
    const away = t.top > a.top ? t.top - (a.top + a.height) : a.top - (t.top + t.height);
    return Math.min(5, Math.max(0, away + 16) / 90);
  }

  /** The words of line `text` with where each starts and ends, in UTF-16 units (JavaScript's own). */
  function words(text: string): { word: string; from: number; to: number }[] {
    const out: { word: string; from: number; to: number }[] = [];
    for (const m of text.matchAll(/\S+\s*/g)) out.push({ word: m[0], from: m.index, to: m.index + m[0].length });
    return out;
  }
  /** How much of a word from `from` to `to` is sung, 0 to 1, with `cut` units of its line sung. */
  const part = (from: number, to: number, cut: number) => Math.min(1, Math.max(0, (cut - from) / Math.max(1, to - from)));
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="lyrics {look}"
  class:synced
  id="lyrics"
  bind:clientHeight={height}
  onwheel={wheel}
  onpointerdown={(e) => e.pointerType !== "mouse" && (dragFrom = { y: e.clientY, at: offset })}
  onpointermove={(e) => dragFrom && (browsed = dragFrom.at - (e.clientY - dragFrom.y))}
  onpointerup={() => (dragFrom = null)}
  onpointercancel={() => (dragFrom = null)}
>
  {#if !shown}
    <p class="none">{W.lyricsLoading}</p>
  {:else if shown.lyrics.lines.length === 0}
    <p class="none">{W.noLyrics}</p>
  {:else}
    <div class="lines" class:held={browsed !== null} bind:this={box} style:transform="translateY({-offset}px)">
      {#each shown.lyrics.lines as l, i (i)}
        {@const lit = i === player.line}
        {#if synced}
          <button class="line" class:on={lit} class:back={l.background} class:duet={l.voice === 1} disabled={!taps} data-i={i} style:filter={blur(i) ? `blur(${blur(i).toFixed(2)}px)` : null} onclick={() => void player.tapLine(i)}>
            {#if lit && player.sung >= 0}
              <!-- The sung edge glides through each word as the core's place moves, softly, as the other clients draw it. -->
              {#each words(l.text) as w}{@const p = part(w.from, w.to, player.sung)}<span class="word" class:lift={p > 0 && p < 1} style:--p={p}>{w.word}</span>{/each}
            {:else}{l.text}{/if}
            {#if l.translation}<span class="tr">{l.translation}</span>{/if}
          </button>
        {:else}
          <p class="plain" data-i={i}>{l.text}</p>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .lyrics {
    --quiet: color-mix(in srgb, var(--ink) 36%, transparent);
    --fade: 48px;
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    touch-action: none;
    mask-image: linear-gradient(transparent, #000 var(--fade), #000 calc(100% - var(--fade)), transparent);
  }
  .lines {
    display: flex;
    flex-direction: column;
    gap: 22px;
    padding: 0 22px 100vh;
    will-change: transform;
    transition: transform 560ms cubic-bezier(0.25, 0.8, 0.3, 1);
  }
  .lines.held {
    transition: none;
  }
  .none {
    color: var(--ink2);
    text-align: center;
    padding-top: 30%;
  }
  .line {
    display: block;
    width: 100%;
    text-align: left;
    padding: 0;
    font: 700 22px/1.25 var(--font);
    letter-spacing: -0.01em;
    color: var(--quiet);
    transition:
      color 420ms ease-out,
      filter 640ms ease-out;
  }
  .line:disabled {
    cursor: default;
    opacity: 1;
  }
  .line.on {
    color: var(--ink);
  }
  .back {
    font-weight: 400;
    font-size: 0.72em;
  }
  .duet {
    text-align: right;
  }
  /* A word as it is sung: the ink up to where the line is sung, softly, then the quiet ink. The soft edge
     runs from before the word to past it, so a word not yet sung is all quiet and one sung all ink. */
  .word {
    --feather: 0.3em;
    display: inline-block;
    white-space: pre;
    color: transparent;
    background: linear-gradient(90deg, var(--ink) calc(var(--p) * (100% + 2 * var(--feather)) - 2 * var(--feather)), var(--quiet) calc(var(--p) * (100% + 2 * var(--feather))));
    background-clip: text;
    -webkit-background-clip: text;
    transition: translate 420ms ease-out;
  }
  .word.lift {
    translate: 0 -2px;
    transition: translate 180ms ease-out;
  }
  .tr {
    display: block;
    font-size: 15px;
    font-weight: 500;
    margin-top: 4px;
  }
  .plain {
    font: 600 20px/1.4 var(--font);
    margin: 0;
  }
  /* The full player's: big, the sung line at 36 % of the height. */
  .full {
    --fade: 72px;
  }
  .full .lines {
    gap: 36px;
    padding: 0 0 100vh;
    transition-duration: 640ms;
  }
  .full .line {
    font-size: 44px;
    transition:
      color 500ms ease-out,
      filter 640ms ease-out;
  }
  .full .word.lift {
    translate: 0 -3px;
  }
  .full .plain {
    font-size: 30px;
  }
  .full .none {
    font-size: 26px;
    font-weight: 700;
    text-align: left;
    padding-top: 0;
    margin: calc(36vh - 48px) 0 0;
  }
  .phone .lines {
    gap: 14px;
    padding: 0 4px 100vh;
  }
  .phone .line {
    font-size: 24px;
  }
</style>
