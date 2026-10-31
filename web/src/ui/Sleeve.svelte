<script lang="ts">
  // A plugin's sleeve over the player's cover, when it gave one for this song: a muted loop or a picture.
  import { sleeves } from "../state/sleeve.svelte";

  let { song }: { song: string } = $props();
  const s = $derived(sleeves.current?.song === song ? sleeves.current : null);
  const poster = $derived(s?.poster?.replace("{size}", "1000"));
  let video = $state<HTMLVideoElement>();

  // HLS goes through hls.js, loaded only then, where the browser has Media Source; a phone's Safari plays
  // it by itself.
  $effect(() => {
    const [v, url] = [video, s?.video];
    if (!v || !url) return;
    if (!url.includes(".m3u8") || !("MediaSource" in window)) {
      v.src = url;
      return;
    }
    let stop = () => {};
    void import("hls.js").then(({ default: Hls }) => {
      const hls = new Hls();
      hls.on(Hls.Events.ERROR, (_e, d) => {
        if (d.fatal) sleeves.failed(song, d.response?.code ?? 0);
      });
      hls.loadSource(url);
      hls.attachMedia(v);
      stop = () => hls.destroy();
    });
    return () => stop();
  });
</script>

{#if s?.video}
  <!-- svelte-ignore a11y_media_has_caption -->
  <video bind:this={video} data-sleeve {poster} muted loop autoplay playsinline onerror={() => sleeves.failed(song, 0)}></video>
{:else if s?.still}
  <img data-sleeve src={s.still.replace("{size}", "1000")} alt="" />
{/if}

<style>
  video,
  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    border-radius: var(--r-cover);
  }
</style>
