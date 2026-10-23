<script lang="ts">
  // The jam over the queue, as Android has it: a guest's jam (whose it is, Listen here, Leave, its own
  // requests waiting), or the requests to the jam this browser hosts. Hosting is in the devices pane.
  import { ask } from "../rpc";
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  const j = $derived(remote.jam);
  const mine = $derived(j?.guest ? j.pending.filter((p) => p.fromId === j.you) : []);
  const decide = (request: number, accept: boolean) => void remote.act(() => ask("jam_decide", request, accept));
  $effect(() => void remote.read());
</script>

{#if j?.guest}
  <div class="jam-head" id="jam">
    <div class="line">
      <Icon name="community" />
      <span class="name clip">{j.host ? W.jamOf(j.host) : W.jam}</span>
      {#if j.along || j.listening !== "Watching"}
        <button class="pill small" id="jam-listen" onclick={() => void remote.listen(j.listening === "Watching")}>
          <Icon name="computer" />{j.listening === "Playing" ? W.jamPlayingHere : W.jamListenHere}
        </button>
      {/if}
      <button class="glyph" aria-label={W.jamLeave} title={W.jamLeave} onclick={() => void remote.leave()}><Icon name="log-out" /></button>
    </div>
    <p class="sub">{W.jamListening(j.members.filter((m) => m.role !== "Host").length)}</p>
    {#if j.listening === "Watching" && j.along}<p class="sub">{W.jamWatching}</p>{/if}
    {#if j.listening === "HostOff"}<p class="sub">{W.jamHostOff}</p>{/if}
    {#if mine.length}
      <h4>{W.jamYoursWaiting}</h4>
      {#each mine as p (p.request)}
        <div class="request">
          <span class="art"><Cover src={p.song.cover} /></span>
          <div class="text"><span class="clip">{p.song.title}</span><span class="sub clip">{W.jamWaitingForHost}</span></div>
        </div>
      {/each}
    {/if}
  </div>
{:else if j?.hosting && j.pending.length}
  <div class="jam-head" id="jam">
    <h4>{W.jamRequests}</h4>
    {#each j.pending as p (p.request)}
      <div class="request">
        <span class="art"><Cover src={p.song.cover} /></span>
        <div class="text">
          <span class="clip">{p.song.title}</span>
          <span class="sub clip">{W.jamAskedBy(p.from)}</span>
          {#if p.provider}<span class="sub">{W.jamWillDownload}</span>{/if}
        </div>
        <button class="glyph" aria-label={W.decline} title={W.decline} onclick={() => decide(p.request, false)}><Icon name="xmark" /></button>
        <button class="glyph solid" aria-label={W.accept} title={W.accept} onclick={() => decide(p.request, true)}><Icon name="check" /></button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .jam-head {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 4px 8px 12px;
    border-bottom: 1px solid var(--hair);
    margin-bottom: 4px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .line .name {
    flex: 1;
    font-size: 16px;
    font-weight: 700;
  }
  h4 {
    font-size: 13px;
    margin: 8px 0 2px;
  }
  .sub {
    margin: 0;
  }
  .request {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 0;
  }
  .art {
    width: 40px;
    height: 40px;
    flex: none;
  }
  .art :global(.cover) {
    border-radius: var(--r-cover);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .glyph.solid {
    background: var(--ink);
    color: var(--page);
  }
</style>
