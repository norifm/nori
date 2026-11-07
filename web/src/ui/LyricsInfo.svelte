<script lang="ts">
  // Where the playing song's lyrics came from and how they are timed, with the timing a step earlier or
  // later (kept for those words) or back to the check's: the shell's dialog card, its lines on a group.
  import { ask } from "../rpc";
  import { player } from "../state/player.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let dialog = $state<HTMLDialogElement>();
  const shown = $derived(player.shown);
  $effect(() => dialog?.showModal());
  const close = () => (ui.lyricsInfo = false);
  /** A step sooner (> 0), later (< 0), or back to the check's timing (0); the core's `lyrics_nudge`. */
  const nudge = (dir: number) => void ask("lyrics_nudge", dir);
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog class="card" id="lyrics-info" bind:this={dialog} onclose={close} onclick={(e) => e.target === dialog && close()}>
  <h2>{W.lyricsInfo}</h2>
  {#if shown}
    <div class="group">
      <div class="group-row"><span class="words">{W.lyricsFrom}</span><span class="value">{W.origins[shown.origin] ?? shown.origin}</span></div>
      <div class="group-row">
        <span class="words"><span>{W.lyricsTiming}</span><span class="sub timing">{W.timing(shown.timing)}</span></span>
        {#if shown.lyrics.synced}
          <button class="glyph" aria-label={W.lyricsLater} title={W.lyricsLater} onclick={() => nudge(-1)}><Icon name="minus" /></button>
          <button class="glyph" aria-label={W.lyricsSooner} title={W.lyricsSooner} onclick={() => nudge(1)}><Icon name="plus" /></button>
        {/if}
      </div>
    </div>
    {#if shown.lyrics.synced && shown.timing.kind === "Yours"}
      <div class="group"><button class="group-row" onclick={() => nudge(0)}><span class="words">{W.lyricsReset}</span></button></div>
    {/if}
  {:else}
    <p class="sub">{W.noLyrics}</p>
  {/if}
  <div class="end"><button class="text-action" onclick={close}>{W.done}</button></div>
</dialog>

<style>
  .timing {
    font-variant-numeric: tabular-nums;
    white-space: normal !important;
  }
  .glyph {
    width: 40px;
    height: 40px;
    margin: -4px -6px;
  }
</style>
