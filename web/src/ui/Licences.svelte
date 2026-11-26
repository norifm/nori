<script lang="ts">
  // The licenses page, as Android's: every credit the core lists for the web (`about`), grouped, its license
  // at the end of the row. A click shows whose it is and the license's full text, as the licenses ask to be
  // shipped; the texts are Android's bundled ones, each loaded when it is first shown.
  import { ask } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { W } from "../words";
  import Page from "./Page.svelte";

  type Credit = { name: string; what: string; copyright: string; licence: string; file: string | null };
  const page = reading("settings", async () => JSON.parse(await ask<string>("about")) as { credits: { group: string; list: Credit[] }[] });

  const texts = import.meta.glob<string>("../../../android/app/src/main/assets/licences/*.txt", { query: "?raw", import: "default" });
  const textOf = async (file: string) => (await texts[`../../../android/app/src/main/assets/licences/${file}.txt`]?.())?.trim() ?? null;

  let dialog = $state<HTMLDialogElement>();
  let open = $state.raw<Credit | null>(null);
  let text = $state<string | null>(null);
  $effect(() => {
    if (open) dialog?.showModal();
  });
  async function show(c: Credit) {
    text = c.file ? "" : W.noLicenceText;
    open = c;
    if (c.file) text = (await textOf(c.file).catch(() => null)) ?? W.licenceUnreadable;
  }
</script>

<Page title={W.licences} back narrow ready={!!page.data} error={page.error}>
  {#each page.data?.credits ?? [] as g (g.group)}
    <div class="group-label">{W.creditGroups[g.group] ?? g.group}</div>
    <div class="group" data-credits={g.group}>
      {#each g.list as c (c.name)}
        <button class="group-row" onclick={() => void show(c)}><span class="words"><span>{c.name}</span><span class="sub">{c.what}</span></span><span class="value">{c.licence}</span></button>
      {/each}
    </div>
  {/each}
</Page>

{#if open}
  <dialog class="card" id="licence" bind:this={dialog} onclose={() => (open = null)}>
    <h2>{open.name}</h2>
    <div class="body">
      <p>{open.copyright}</p>
      <p class="licence">{open.licence}</p>
      <pre>{text}</pre>
    </div>
    <div class="end"><button type="button" class="text-action" onclick={() => dialog?.close()}>{W.close}</button></div>
  </dialog>
{/if}

<style>
  .group {
    margin: 0 -4px;
  }
  .group-row .value {
    flex-shrink: 0;
  }
  .body {
    overflow-y: auto;
    min-height: 0;
    font-size: 14px;
  }
  .licence {
    margin: 4px 0 12px;
    font-size: 12px;
    color: var(--ink2);
  }
  pre {
    margin: 0;
    white-space: pre-wrap;
    font-size: 12px;
    color: var(--ink2);
  }
</style>
