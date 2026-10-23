<script lang="ts">
  // The queue another device or app left on the server, offered at the top of Home as Android offers it:
  // a card on the season's paper dyed with the song's cover, the song, where it stands, Resume and a close.
  // Taken or closed, that queue is not offered again (the core's `queue_offer_done`).
  import { ask } from "../rpc";
  import { coverLook } from "../state/coverLook";
  import { player } from "../state/player.svelte";
  import { looks } from "../state/theme.svelte";
  import type { Song } from "../state/types";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  type Offer = { songs: Song[]; index: number; positionMs: number };
  let offer = $state.raw<Offer | null>(null);
  let vars = $state("");
  // Asked each time Home is drawn, while nothing plays here.
  void ask<string>("queue_offer", player.playing, player.song?.id ?? null).then((j) => (offer = JSON.parse(j)), () => {});
  const song = $derived(offer?.songs[offer.index]);
  $effect(() => {
    void looks.rev;
    const src = song?.cover;
    if (src) void coverLook(src).then((l) => (vars = l?.vars ?? ""));
  });

  function done(o: Offer) {
    offer = null;
    void ask("queue_offer_done", JSON.stringify(o.songs), o.index, o.positionMs);
  }

  async function take(o: Offer) {
    done(o);
    await player.play(o.songs, o.index, false, "", "");
    await player.seek(o.positionMs);
  }
</script>

{#if offer && song}
  {@const o = offer}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="continue tinted" style={vars} onclick={() => void take(o)} id="continue">
    <div class="art"><Cover src={song.cover} radius="0" /></div>
    <div class="words">
      <span class="label">{W.continueLabel}</span>
      <span class="title">{song.title}</span>
      <span class="place clip">{W.offerPlace(song.artist, o.index + 1, o.songs.length)}</span>
      <button class="resume"><Icon name="play-solid" />{W.resume}</button>
    </div>
    <button
      class="close"
      aria-label={W.close}
      title={W.close}
      onclick={(e) => {
        e.stopPropagation();
        done(o);
      }}><Icon name="xmark" /></button
    >
  </div>
{/if}

<style>
  .continue {
    position: relative;
    display: flex;
    height: 178px;
    margin-top: 16px;
    max-width: 560px;
    border-radius: var(--r-tile);
    overflow: hidden;
    background: var(--page);
    color: var(--ink);
    cursor: pointer;
  }
  /* The cover, going into the card over its right half. */
  .art {
    position: relative;
    width: 178px;
    flex: none;
  }
  .art::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(to right, transparent 55%, var(--page));
  }
  .art :global(.cover) {
    height: 100%;
  }
  .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding: 16px 16px 14px 6px;
  }
  .label {
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--ink) 65%, transparent);
  }
  .title {
    margin-top: 6px;
    font: 600 21px/24px var(--display);
    letter-spacing: -0.3px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .place {
    margin-top: 2px;
    font-size: 13px;
    color: color-mix(in srgb, var(--ink) 70%, transparent);
  }
  .resume {
    margin-top: auto;
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 38px;
    padding: 0 14px;
    border-radius: 999px;
    background: var(--ink);
    color: var(--page);
    font-size: 14px;
    font-weight: 600;
  }
  .resume :global(.icon) {
    width: 18px;
    height: 18px;
  }
  .close {
    position: absolute;
    top: 0;
    right: 0;
    width: 40px;
    height: 40px;
    display: grid;
    place-items: center;
    color: color-mix(in srgb, var(--ink) 60%, transparent);
  }
  .close :global(.icon) {
    width: 18px;
    height: 18px;
  }
</style>
