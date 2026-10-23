<script lang="ts">
  // The jam in the devices pane, as Android's devices sheet has it: Start a Jam, or Your Jam with Invite
  // (the code and the link), its settings, who is in (admins made and people removed here) and End; and
  // joining someone's. A guest's jam is over its queue. The rules are the core's; this draws them.
  import { ask, playerUp } from "../rpc";
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  const j = $derived(remote.jam);
  let inviting = $state(false);
  let copied = $state(false);
  let link = $state("");
  let name = $state("");
  let said = $state("");
  let qr = $state<{ size: number; dark: string } | null>(null);
  const guests = $derived(j?.members.filter((m) => m.role !== "Host") ?? []);

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
  $effect(() => {
    const l = j?.link;
    if (inviting && l) void ask<string>("qr", l).then((q) => (qr = JSON.parse(q)));
  });
  /** One square per dark module, as one path. */
  const path = (q: { size: number; dark: string }) => {
    let d = "";
    for (let i = 0; i < q.dark.length; i++) if (q.dark[i] === "1") d += `M${i % q.size} ${Math.floor(i / q.size)}h1v1h-1z`;
    return d;
  };
</script>

<div class="jam" id="jam-host">
  {#if !player.up || !j}
    <!-- The player module runs the remote: it loads in the background, or now. -->
    <button class="pill solid" onclick={act(playerUp)}><Icon name="community" />{W.jamStart}</button>
  {:else if j.hosting && inviting}
    <button class="pill small" onclick={() => (inviting = false)}><Icon name="arrow-left" />{W.back}</button>
    <h3>{W.jamInviteTitle}</h3>
    {#if qr}
      <svg class="qr" viewBox="-2 -2 {qr.size + 4} {qr.size + 4}" role="img" aria-label={W.jamInviteTitle}><rect x="-2" y="-2" width={qr.size + 4} height={qr.size + 4} fill="white" /><path d={path(qr)} fill="black" /></svg>
    {/if}
    <p class="sub">{W.jamInviteHint}</p>
    <div class="invite">
      <input class="field link" readonly value={j.link ?? ""} aria-label={W.jamLink} />
      <button class="pill" onclick={() => void navigator.clipboard?.writeText(j.link ?? "").then(() => (copied = true))}>{copied ? W.copied : W.copyLink}</button>
    </div>
  {:else if j.hosting}
    <div class="line">
      <Icon name="community" /><span class="name">{W.jamYours}</span><span class="sub">{W.jamListening(guests.length)}</span>
      <button class="round end" aria-label={W.jamEnd} title={W.jamEnd} onclick={act(() => ask("jam_close"))}><Icon name="log-out" /></button>
    </div>
    <button class="pill solid" id="jam-invite" onclick={() => (inviting = true)}><Icon name="user-plus" />{W.jamInvite}</button>
    <label class="row">
      <span>{W.jamAlong}</span>
      <input type="checkbox" class="switch" checked={j.along} onchange={(e) => void act(() => ask("jam_along", e.currentTarget.checked))()} />
    </label>
    <label class="row">
      <span>{W.jamRemoteSongs}<span class="sub line2">{W.jamRemoteSongsLine}</span></span>
      <input type="checkbox" class="switch" checked={j.remoteSongs} onchange={(e) => void act(() => ask("jam_remote_songs", e.currentTarget.checked))()} />
    </label>
    <h4>{W.jamPeople}</h4>
    <ul class="members">
      {#each guests as m (m.id)}
        <li>
          <Icon name="user" /><span class="clip who">{m.name}<span class="sub line2">{W.roles[m.role] ?? m.role}</span></span>
          <button class="round" class:lit={m.role === "Admin"} aria-pressed={m.role === "Admin"} aria-label={m.role === "Admin" ? W.jamMakeGuest : W.jamMakeAdmin} title={m.role === "Admin" ? W.jamMakeGuest : W.jamMakeAdmin} onclick={act(() => ask("jam_promote", m.id, m.role !== "Admin"))}><Icon name="key" /></button>
          <button class="round" aria-label={W.jamSendOut} title={W.jamSendOut} onclick={act(() => ask("jam_kick", m.id))}><Icon name="xmark" /></button>
        </li>
      {:else}
        <li class="sub">{W.jamNoOne}</li>
      {/each}
    </ul>
  {:else if !j.guest}
    {#if j.start === "Unsupported"}
      <p class="sub">{W.jamUnavailable}</p>
    {:else if j.start !== "Unavailable"}
      <button class="pill solid" id="jam-start" disabled={j.start === "Starting"} onclick={act(() => ask("jam_open"))}><Icon name="community" />{j.start === "Starting" ? W.jamStarting : W.jamStart}</button>
    {/if}
    <h4>{W.jamJoin}</h4>
    <input class="field" id="jam-link" bind:value={link} placeholder={W.jamPaste} aria-label={W.jamLink} />
    <input class="field" id="jam-name" bind:value={name} placeholder={W.jamName} aria-label={W.jamName} />
    <button class="pill" id="jam-join" onclick={join}>{W.jamJoin}</button>
  {/if}
  {#if said}<p class="sub">{said}</p>{/if}
</div>

<style>
  .jam {
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: stretch;
    padding: 12px 4px 16px;
    border-top: 1px solid var(--hair);
    margin-top: 8px;
  }
  .jam > .pill {
    align-self: flex-start;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .lit {
    color: var(--accent);
  }
  .line .name {
    flex: 1;
    font-size: 16px;
    font-weight: 700;
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
  .line2 {
    display: block;
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
  .qr {
    width: 232px;
    height: 232px;
    align-self: center;
    border-radius: 16px;
    shape-rendering: crispEdges;
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
  .who {
    flex: 1;
  }
</style>
