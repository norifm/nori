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

  /** The sung part of line `i`'s text, in UTF-16 units (JavaScript's own). */
  const sungPart = (i: number, text: string) => (i === player.line && player.sung > 0 ? Math.min(text.length, Math.floor(player.sung)) : -1);
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
        <button class="line" class:on={i === player.line} disabled={!taps} class:past={i < player.line} class:duet={l.voice === 1} data-i={i} onclick={() => void player.tapLine(i)}>
          {#if cut >= 0}<span class="sung">{l.text.slice(0, cut)}</span><span class="rest">{l.text.slice(cut)}</span>{:else}{l.text || "♪"}{/if}
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
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 24px 4px 50vh;
    scrollbar-width: none;
    mask-image: linear-gradient(transparent, #000 6%, #000 80%, transparent);
  }
  .none {
    color: var(--dim);
    text-align: center;
    padding-top: 30%;
  }
  .line {
    display: block;
    width: 100%;
    text-align: left;
    padding: 8px 10px;
    margin: 0 0 4px;
    border-radius: 10px;
    font: 700 26px/1.25 var(--display);
    letter-spacing: -0.01em;
    color: var(--faint);
    transition: color 0.25s, transform 0.3s;
    transform-origin: left center;
  }
  .line:disabled {
    cursor: default;
    opacity: 1;
  }
  .line:hover:enabled {
    background: var(--hover);
  }
  .line.on {
    color: var(--text);
  }
  .line.on .rest {
    color: var(--faint);
  }
  .line.on .sung {
    color: var(--text);
  }
  .line.past {
    color: var(--dim);
    opacity: 0.6;
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
    font: 600 20px/1.4 var(--display);
    margin: 0 0 14px;
    padding: 0 10px;
  }
  @media (max-width: 899px) {
    .line {
      font-size: 24px;
    }
  }
</style>
