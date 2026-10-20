<script lang="ts">
  // The lyrics: the sung line lit (and in it the words sung so far, where the lyrics time their words)
  // by the core's lyric clock, asked again only when the clock says the next change comes; nothing runs
  // while the song holds or the page is hidden. A click on a line plays from it.
  import { player } from "../state/player.svelte";
  import { W } from "../words";

  let box = $state<HTMLDivElement>();
  const shown = $derived(player.shown && player.shown.song === player.song?.id ? player.shown : null);
  const synced = $derived(!!shown?.lyrics.synced);
  /** Lines play from where they are sung on a click, unless a jam guest asks the host for all it plays. */
  const taps = $derived(synced && !player.now?.asks);

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

  // The lit line kept a little above the middle.
  $effect(() => {
    const el = box?.querySelector<HTMLElement>(`[data-i="${player.line}"]`);
    if (!box || !el) return;
    box.scrollTo({ top: el.offsetTop - box.clientHeight * 0.38, behavior: "smooth" });
  });

  /** How much of line `i`'s text is sung, in fractional UTF-16 units (JavaScript's own); -1 while it is
   * not swept. A swept line is dim from its first frame, not lit whole until its first letter. */
  const sungPart = (i: number, text: string) => (i === player.line && player.sung >= 0 ? Math.min(text.length, player.sung) : -1);

  /** Letters the sung edge spans: the line lights through a soft gradient this wide, as the other clients
   * draw it, rather than a letter at a time. */
  const SOFT = 3;

  /** `text`'s letters (whole code points) with how lit each is, 0 to 1, for `cut` sung units. */
  function swept(text: string, cut: number): { ch: string; lit: number }[] {
    const out: { ch: string; lit: number }[] = [];
    let at = 0;
    for (const ch of text) {
      out.push({ ch, lit: Math.min(1, Math.max(0, (cut - at) / SOFT + 0.5)) });
      at += ch.length;
    }
    return out;
  }
</script>

<div class="lyrics" class:synced id="lyrics" bind:this={box}>
  {#if !shown}
    <p class="none">{W.lyricsLoading}</p>
  {:else if shown.lyrics.lines.length === 0}
    <p class="none">{W.noLyrics}</p>
  {:else}
    {#each shown.lyrics.lines as l, i (i)}
      {@const cut = sungPart(i, l.text)}
      {#if synced}
        <button class="line" class:on={i === player.line} disabled={!taps} class:duet={l.voice === 1} data-i={i} onclick={() => void player.tapLine(i)}>
          {#if cut >= 0}
            <!-- The sung edge glides through the line as the core's place moves, a few letters soft. -->
            {#each swept(l.text, cut) as s}<span style="color: color-mix(in srgb, var(--ink) {s.lit * 100}%, var(--quiet))">{s.ch}</span>{/each}
          {:else}{l.text || "♪"}{/if}
          {#if l.translation}<span class="tr">{l.translation}</span>{/if}
        </button>
      {:else}
        <p class="plain">{l.text}</p>
      {/if}
    {/each}
  {/if}
</div>

<style>
  .lyrics {
    --quiet: color-mix(in srgb, var(--ink2) 50%, transparent);
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 24px 4px 50vh;
    scrollbar-width: none;
    mask-image: linear-gradient(transparent, #000 6%, #000 80%, transparent);
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
    padding: 8px 10px;
    margin: 0 0 4px;
    border-radius: 12px;
    font: 700 24px/1.25 var(--font);
    letter-spacing: -0.01em;
    color: var(--quiet);
    transition: color 0.25s;
  }
  .line:disabled {
    cursor: default;
    opacity: 1;
  }
  .line:hover:enabled {
    background: var(--hover);
  }
  .line.on {
    color: var(--ink);
  }
  .duet {
    text-align: right;
  }
  .tr {
    display: block;
    font-size: 15px;
    font-weight: 500;
    margin-top: 4px;
  }
  .plain {
    font: 600 20px/1.4 var(--font);
    margin: 0 0 14px;
    padding: 0 10px;
  }
</style>
