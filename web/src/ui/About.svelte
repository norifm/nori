<script lang="ts">
  // About, as Android's: what this is and what it is built out of. Most of a bug report is answered here
  // (which version, which browser), and a click on the first row copies all of it. Then what it runs on, and its license with the page of every credit
  // (`#/settings/About/licenses`).
  import { href } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { version }: { version: string } = $props();
  const title = $derived(W.aboutTitle(version));
  const build = W.aboutBuild(navigator.userAgent);
  const lines: [string, string][] = [
    [W.playback, W.aboutPlayback],
    [W.libraryAndSearch, W.aboutLibrary],
    [W.automix, W.aboutAutomix],
    [W.interfaceTitle, W.aboutUi],
  ];

  async function copy() {
    await navigator.clipboard?.writeText(W.aboutReport(title, build, lines));
    ui.say(W.copied);
  }
</script>

<div class="group-label">nori</div>
<div class="group" id="about">
  <button class="group-row" data-row="version" onclick={() => void copy()}
    ><span class="words"><span>{title}</span><span class="sub">{build}</span></span><span class="value">{W.copyIt}</span></button
  >
</div>
<div class="group-label">{W.underTheHood}</div>
<div class="group">
  {#each lines as [what, line] (what)}
    <div class="group-row"><span class="words"><span>{what}</span><span class="sub">{line}</span></span></div>
  {/each}
</div>
<div class="group-label">{W.openSource}</div>
<div class="group">
  <div class="group-row"><span class="words"><span>{W.freeSoftware}</span><span class="sub">{W.licenceLine}</span></span><span class="value">AGPL-3.0</span></div>
  <a class="group-row" href="{href('settings', 'About')}/licenses" data-link="licenses"
    ><span class="words"><span>{W.licences}</span><span class="sub">{W.licencesDetail}</span></span><Icon name="nav-arrow-right" /></a
  >
</div>

<style>
  .group {
    margin: 0 -4px;
  }
</style>
