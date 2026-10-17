<script lang="ts">
  // The jam: start one or join someone's; the invite, requests and who is in while hosting; the host and
  // listening along as a guest. The rules are the core's; this draws them and asks.
  import { ask, playerUp } from "../rpc";
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  const j = $derived(remote.jam);
  let link = $state("");
  let name = $state("");
  let said = $state("");
  let copied = $state(false);

  const act = (f: () => Promise<unknown>) => async () => {
    said = "";
    try {
      await remote.act(f);
    } catch (e) {
      said = W.failed(String((e as Error).message ?? e));
    }
  };
  const join = act(async () => {
    const r = await remote.join(link, name);
    if (r !== "joined") said = r === "own" ? W.jamOwn : W.jamEnded;
  });
  $effect(() => void remote.read());
</script>

<div class="jam" id="jam">
  {#if !player.up || !j}
    <!-- The player module runs the remote: it loads in the background, or now. -->
    <p class="sub">{W.jamLead}</p>
    <button class="pill solid" onclick={act(playerUp)}><Icon name="jam" />{W.jamStart}</button>
  {:else if j.guest}
    <h3>{W.jamOf(j.host ?? "")}</h3>
    <p class="sub">{j.asks ? W.jamAsks : W.jamAdds}</p>
    <label class="row">
      <span>{W.jamListen}</span>
      <input type="checkbox" class="switch" id="jam-listen" checked={j.listening === "Playing"} onchange={(e) => void act(() => remote.listen(e.currentTarget.checked))()} />
    </label>
    {#if j.listening === "HostOff"}<p class="sub">{W.jamHostOff}</p>{/if}
    {@render people()}
    <button class="pill" onclick={act(() => remote.leave())}>{W.jamLeave}</button>
  {:else if j.hosting}
    <h3>{W.jamYours}</h3>
    <div class="invite">
      <input class="field link" readonly value={j.link ?? ""} aria-label={W.jamLink} />
      <button class="pill" onclick={() => void navigator.clipboard?.writeText(j.link ?? "").then(() => (copied = true))}>{copied ? W.copied : W.copy}</button>
    </div>
    <label class="row">
      <span>{W.jamAlong}</span>
      <input type="checkbox" class="switch" checked={j.along} onchange={(e) => void act(() => ask("jam_along", e.currentTarget.checked))()} />
    </label>
    {#if j.pending.length}
      <h4>{W.jamRequests}</h4>
      {#each j.pending as p (p.request)}
        <div class="request">
          <div class="text"><span class="clip">{p.title}</span><span class="sub clip">{p.artist} · {W.jamAskedBy(p.from)}</span></div>
          <button class="pill small solid" onclick={act(() => ask("jam_decide", p.request, true))}>{W.accept}</button>
          <button class="pill small" onclick={act(() => ask("jam_decide", p.request, false))}>{W.decline}</button>
        </div>
      {/each}
    {/if}
    {@render people()}
    <button class="pill" onclick={act(() => ask("jam_close"))}>{W.jamEnd}</button>
  {:else}
    <p class="sub">{W.jamLead}</p>
    {#if j.start === "Unavailable"}
      <p class="sub">{W.jamUnavailable}</p>
    {:else}
      <button class="pill solid" id="jam-start" disabled={j.start === "Starting"} onclick={act(() => ask("jam_open"))}><Icon name="jam" />{j.start === "Starting" ? W.jamStarting : W.jamStart}</button>
    {/if}
    <h4>{W.jamJoin}</h4>
    <input class="field" id="jam-link" bind:value={link} placeholder={W.jamPaste} aria-label={W.jamLink} />
    <input class="field" id="jam-name" bind:value={name} placeholder={W.jamName} aria-label={W.jamName} />
    <button class="pill" id="jam-join" onclick={join}>{W.jamJoin}</button>
  {/if}
  {#if said}<p class="sub">{said}</p>{/if}
</div>

{#snippet people()}
  {#if j && j.members.length}
    <h4>{W.jamPeople}</h4>
    <ul class="members">
      {#each j.members as m (m.id)}
        <li><Icon name={m.role === "Host" ? "jam" : "artist"} /><span class="clip">{m.name}</span><span class="sub">{W.roles[m.role] ?? m.role}</span></li>
      {/each}
    </ul>
  {/if}
{/snippet}

<style>
  .jam {
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: stretch;
  }
  .jam > .pill {
    align-self: flex-start;
  }
  h3 {
    font-size: 19px;
  }
  h4 {
    font-size: 14px;
    margin-top: 8px;
  }
  .sub {
    margin: 0;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 14px;
  }
  .invite {
    display: flex;
    gap: 8px;
  }
  .request {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .members {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .members li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 0;
  }
  .members li .clip {
    flex: 1;
  }
</style>
