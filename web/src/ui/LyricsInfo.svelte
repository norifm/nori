<script lang="ts">
  // Where the playing song's lyrics came from and how they are timed, with the timing a step earlier or
  // later (kept for those words) or back to the check's.
  import { ask } from "../rpc";
  import { player } from "../state/player.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let dialog = $state<HTMLDialogElement>();
  const shown = $derived(player.shown);
  $effect(() => dialog?.showModal());
  const close = () => (ui.lyricsInfo = false);
  const nudge = (dir: number) => void ask("lyrics_nudge", dir);
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog class="info" id="lyrics-info" bind:this={dialog} onclose={close} onclick={(e) => e.target === dialog && close()}>
  <h3>{W.lyricsInfo}</h3>
  {#if shown}
    <div class="line"><span class="sub">{W.lyricsFrom}</span><span>{W.origins[shown.origin] ?? shown.origin}</span></div>
    <div class="line"><span class="sub">{W.lyricsTiming}</span><span class="timing">{W.timing(shown.timing)}</span></div>
    {#if shown.lyrics.synced}
      <div class="stepper">
        <button class="pill quiet" aria-label={W.lyricsLater} onclick={() => nudge(-1)}><Icon name="minus-circle" />{W.lyricsLater}</button>
        <button class="pill quiet" aria-label={W.lyricsSooner} onclick={() => nudge(1)}><Icon name="plus-circle" />{W.lyricsSooner}</button>
      </div>
      <button class="pill small quiet reset" disabled={shown.timing.kind !== "Yours"} onclick={() => nudge(0)}>{W.lyricsReset}</button>
    {/if}
  {:else}
    <p class="sub">{W.noLyrics}</p>
  {/if}
  <button class="pill done" onclick={close}>{W.done}</button>
</dialog>

<style>
  .info {
    border: 1px solid var(--hair);
    padding: 22px;
    border-radius: 22px;
    background: var(--surface);
    color: var(--ink);
    box-shadow: var(--shadow);
    width: min(380px, calc(100vw - 32px));
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .info:not([open]) {
    display: none;
  }
  .info::backdrop {
    background: var(--scrim);
  }
  h3 {
    font-size: 19px;
    font-weight: 700;
  }
  .line {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    font-size: 14px;
  }
  .timing {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .stepper {
    display: flex;
    gap: 8px;
  }
  .stepper .pill {
    flex: 1;
  }
  .reset {
    align-self: center;
  }
  .done {
    margin-top: 6px;
  }
</style>
