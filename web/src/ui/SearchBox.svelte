<script lang="ts">
  // The top bar's search: a field that widens when focused and lists the best matches under it as you
  // type; Enter opens the whole search page. Read a moment after the typing stops.
  import { read } from "../rpc";
  import { href, route, searchFor } from "../state/route.svelte";
  import { player } from "../state/player.svelte";
  import type { Album, Artist, Song } from "../state/types";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  type Found = { artists?: Artist[]; albums?: Album[]; songs?: Song[] };
  let query = $state("");
  // The search page's query, however it was reached (typed, a link, Back).
  $effect(() => {
    if (route.top === "search") query = route.arg;
  });
  let found = $state.raw<Found | null>(null);
  let open = $state(false);
  let box = $state<HTMLDivElement>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let asked = 0;

  function typed() {
    clearTimeout(timer);
    const q = query.trim();
    // A read still out when the box is cleared is no longer wanted.
    const mine = ++asked;
    if (!q) {
      found = null;
      open = false;
      return;
    }
    timer = setTimeout(async () => {
      const p = await read<Found>({ kind: "search", query: q }).catch(() => null);
      if (mine === asked) found = p;
    }, 200);
    open = true;
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    open = false;
    if (query.trim()) searchFor(query);
  }

  const none = $derived(found && !found.artists?.length && !found.albums?.length && !found.songs?.length);
  const close = () => (open = false);
</script>

<svelte:window onpointerdown={(e) => open && !box?.contains(e.target as Node) && close()} onkeydown={(e) => e.key === "Escape" && close()} />

<div class="box" bind:this={box}>
  <form class="chrome search" class:on={route.tab === "search"} role="search" onsubmit={submit}>
    <Icon name="search" />
    <input type="search" placeholder={W.searchHint} aria-label={W.search} autocomplete="off" bind:value={query} oninput={typed} onfocus={() => query.trim() && (open = true)} />
  </form>
  {#if open && found}
    <div class="results" role="listbox">
      {#if none}<p class="sub empty">{W.noResults}</p>{/if}
      {#each (found.artists ?? []).slice(0, 2) as a (a.id)}
        <a class="res" href={href("artist", a.id)} onclick={close}><span class="art"><Cover src={a.cover} round fallback="user" /></span><span class="clip">{a.name}</span></a>
      {/each}
      {#each (found.albums ?? []).slice(0, 3) as a (a.id)}
        <a class="res" href={href("album", a.id)} onclick={close}><span class="art"><Cover src={a.cover} fallback="compact-disc" /></span><span class="clip">{a.name}<span class="sub">{a.artist}</span></span></a>
      {/each}
      {#each (found.songs ?? []).slice(0, 4) as s, i (s.id)}
        <button class="res" onclick={() => (close(), void player.play(found?.songs?.slice(i, i + 1) ?? [], 0))}><span class="art"><Cover src={s.cover} /></span><span class="clip">{s.title}<span class="sub">{s.artist}</span></span></button>
      {/each}
      {#if !none}<a class="res more" href={href("search", query.trim())} onclick={close}>{W.moreResults}</a>{/if}
    </div>
  {/if}
</div>

<style>
  .box {
    position: relative;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 220px;
    height: 50px;
    padding: 0 16px;
    border-radius: 25px;
    transition: width 200ms;
    color: color-mix(in srgb, var(--ink) 65%, transparent);
  }
  .search.on {
    color: var(--accent);
  }
  .search:focus-within {
    width: 320px;
    border: 1px solid var(--accent);
  }
  .search :global(.icon) {
    width: 21px;
    height: 21px;
  }
  input {
    all: unset;
    flex: 1;
    min-width: 0;
    color: var(--ink);
    font-size: 15px;
  }
  input::placeholder {
    color: var(--ink2);
  }
  input::-webkit-search-cancel-button {
    display: none;
  }
  .results {
    position: absolute;
    top: 58px;
    right: 0;
    width: 360px;
    padding: 6px;
    border-radius: var(--r-tile);
    background: var(--surface);
    border: var(--hairline) solid var(--hair);
    display: flex;
    flex-direction: column;
    z-index: 5;
  }
  .res {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 8px;
    border-radius: var(--r-card);
    text-align: left;
    font-size: 15px;
  }
  .res:hover {
    background: var(--hover);
  }
  .res .clip {
    display: flex;
    flex-direction: column;
    flex: 1;
  }
  .art {
    width: 40px;
    flex: none;
  }
  .more {
    color: var(--ink2);
    justify-content: center;
    font-size: 14px;
    min-height: 40px;
  }
  .empty {
    margin: 0;
    padding: 12px;
  }
</style>
