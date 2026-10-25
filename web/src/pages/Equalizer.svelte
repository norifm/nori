<script lang="ts">
  // The equalizer, as the desktop's: the curve it plays with a point for each band (drag one, or click it
  // to edit it below; clicking elsewhere adds a parametric band), then presets, pre-amp and the output's
  // balance, mono and limiter. What it draws comes from the core (crates/web/src/eq.rs); every edit
  // answers with the page as it is then.
  import { ask } from "../rpc";
  import { route } from "../state/route.svelte";
  import { ui } from "../state/ui.svelte";
  import type { Equalizer } from "../state/types";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";

  let eq = $state<Equalizer | null>(null);
  let failed = $state<string | undefined>();
  let chosen = $state(-1);
  let held = $state(-1);
  let w = $state(0);
  const h = 240;
  /** Room left and right of the curve, so the end points are whole. */
  const inset = 14;

  async function load() {
    try {
      eq = JSON.parse(await ask<string>("equalizer"));
    } catch (e) {
      failed = String(e);
    }
  }
  $effect(() => {
    void route.visit;
    void load();
  });

  // One edit to the worker at a time; while one is out only the latest waiting is kept.
  let busy = false;
  let waiting: (() => Promise<string>) | null = null;
  async function send(edit: () => Promise<string>) {
    if (busy) {
      waiting = edit;
      return;
    }
    busy = true;
    try {
      eq = JSON.parse(await edit());
    } finally {
      busy = false;
    }
    const next = waiting;
    waiting = null;
    if (next) await send(next);
  }

  async function set(name: string, value: string) {
    await ask("setting", name, value);
    await load();
  }

  const across = $derived(Math.max(1, w - inset * 2));
  const half = h / 2 - 3;
  const xOf = (x: number) => inset + x * across;
  const yOf = (db: number) => h / 2 - (Math.max(-(eq?.reach ?? 15), Math.min(eq?.reach ?? 15, db)) / (eq?.reach ?? 15)) * half;
  function trace(r: number[], closed: boolean) {
    const last = Math.max(1, r.length - 1);
    const d = r.map((db, i) => `${i ? "L" : "M"}${(inset + (i / last) * across).toFixed(1)} ${yOf(db).toFixed(1)}`).join(" ");
    return closed && r.length ? `${d} L${(inset + across).toFixed(1)} ${h / 2} L${inset} ${h / 2} Z` : d;
  }
  const band = $derived(eq && chosen >= 0 && chosen < eq.bands.length ? eq.bands[chosen] : null);
  const step = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, Math.round(v * 2) / 2));

  /** Band `i` set to place `x` across the curve and gain `db`, keeping the rest of it. */
  function place(i: number, x: number, db: number, q?: number, kind?: number, channel?: number) {
    const b = eq!.bands[i];
    const [lo, hi] = eq!.ranges.gain;
    void send(() => ask<string>("eq_band", i, x, step(db, lo, hi), q ?? b.q, kind ?? b.kind, channel ?? b.channel));
  }

  /** Where the pointer is over the curve: across (0..1) and dB. */
  function at(e: PointerEvent) {
    const box = (e.currentTarget as SVGElement).getBoundingClientRect();
    const x = Math.max(0, Math.min(1, (e.clientX - box.left - inset) / across));
    const db = ((h / 2 - (e.clientY - box.top)) / half) * (eq?.reach ?? 15);
    return { x, db };
  }

  function down(e: PointerEvent) {
    if (!eq?.on || e.button !== 0) return;
    const point = (e.target as Element).closest("[data-band]");
    if (point) {
      chosen = held = Number(point.getAttribute("data-band"));
      (e.currentTarget as SVGElement).setPointerCapture(e.pointerId);
    } else if (!eq.graphic) {
      const { x, db } = at(e);
      chosen = eq.bands.length;
      const [lo, hi] = eq.ranges.gain;
      void send(() => ask<string>("eq_add", x, step(db, lo, hi)));
    }
    e.preventDefault();
  }
  function move(e: PointerEvent) {
    if (held < 0 || !eq) return;
    const { x, db } = at(e);
    place(held, eq.graphic ? eq.bands[held].x : x, db);
  }
  function up() {
    held = -1;
  }

  function chooseKind(e: MouseEvent) {
    if (!band) return;
    ui.drop(e, W.eq.kinds.map((k, i) => ({ label: k, chosen: i === band.kind, run: () => place(chosen, band.x, band.gain, band.q, i) })));
  }
</script>

<Page title={W.equalizer} back narrow ready={!!eq} error={failed}>
  {#snippet head()}
    {#if eq}<input type="checkbox" class="switch" aria-label={W.equalizer} checked={eq.on} onchange={(e) => void set("eq", e.currentTarget.checked ? "true" : "false")} />{/if}
  {/snippet}
  {#if eq}
    {#if eq.bypass}<p class="bypass">{W.eq.bypass[eq.bypass] ?? ""}</p>{/if}
    <div class="group plate">
      <div class="chips-line">
        <button class="chip" class:on={eq.graphic} onclick={() => !eq!.graphic && void set("eqMode", "GRAPHIC")}>{W.eq.graphic}</button>
        <button class="chip" class:on={!eq.graphic} onclick={() => eq!.graphic && void set("eqMode", "PARAMETRIC")}>{W.eq.parametric}</button>
        {#if eq.graphic}
          <span class="gap"></span>
          {#each eq.layouts as n (n)}
            <button class="chip" class:on={eq.count === n} onclick={() => eq!.count !== n && ((chosen = -1), void set("eqLayout", String(n)))}>{W.eq.bands(n)}</button>
          {/each}
        {/if}
      </div>
      <div class="curve" bind:clientWidth={w}>
        {#if w}
          <svg width={w} height={h} class:off={!eq.on} class:adds={eq.on && !eq.graphic} onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up} role="application" aria-label={W.equalizer}>
            {#each [-12, -6, 0, 6, 12] as d (d)}<line class="grid" class:zero={d === 0} x1={inset} x2={inset + across} y1={yOf(d)} y2={yOf(d)} />{/each}
            <text class="scale" x={inset + 2} y={yOf(12) + 12}>+12</text>
            <text class="scale" x={inset + 2} y={yOf(-12) - 4}>−12</text>
            {#each [{ f: 100, name: "100" }, { f: 1000, name: "1k" }, { f: 10000, name: "10k" }] as g (g.f)}
              {@const x = xOf(Math.log10(g.f / 20) / 3)}
              <line class="grid" x1={x} x2={x} y1="0" y2={h} />
              <text class="scale" x={x + 4} y={h - 6}>{g.name}</text>
            {/each}
            {#each eq.each as e, i (i)}<path class="shape" class:lit={i === chosen && eq.on} d={trace(e, true)} />{/each}
            <path class="area" d={trace(eq.curve, true)} />
            <path class="line" d={trace(eq.curve, false)} />
            {#if held >= 0 && held < eq.bands.length}<line class="held" x1={xOf(eq.bands[held].x)} x2={xOf(eq.bands[held].x)} y1="0" y2={h} />{/if}
            {#each eq.bands as b, i (i)}
              <g data-band={i} class="point" class:lit={eq.on && (i === chosen || i === held)} class:upright={eq.graphic}>
                <circle class="reach" cx={xOf(b.x)} cy={yOf(b.db)} r="14" />
                <circle class="dot" cx={xOf(b.x)} cy={yOf(b.db)} r={eq.on && (i === chosen || i === held) ? 8 : 6} />
              </g>
            {/each}
          </svg>
          {#if held >= 0 && held < eq.bands.length}
            {@const b = eq.bands[held]}
            <div class="bubble" style:left="{Math.max(4, Math.min(w - 130, xOf(b.x) - 55))}px">{eq.graphic ? b.label : b.freq}&ensp;{b.usesGain ? W.eq.db(b.value) : W.eq.kinds[b.kind]}</div>
          {/if}
        {/if}
      </div>
      {#if eq.on && !band}<p class="hint">{eq.graphic ? W.eq.hintGraphic : eq.bands.length ? W.eq.hintParametric : W.eq.hintEmpty}</p>{/if}
      {#if eq.on && band && eq.graphic}
        <div class="editor">
          <div class="tools">
            <button class="glyph quiet" aria-label={W.eq.previous} disabled={chosen <= 0} onclick={() => chosen--}><Icon name="nav-arrow-left" /></button>
            <strong>{band.label}</strong>
            <button class="glyph quiet" aria-label={W.eq.next} disabled={chosen >= eq.bands.length - 1} onclick={() => chosen++}><Icon name="nav-arrow-right" /></button>
            <span class="fill"></span>
            <button class="glyph quiet" aria-label={W.eq.lower} onclick={() => place(chosen, band.x, band.gain - 0.5)}><Icon name="minus-circle" /></button>
            <button class="glyph quiet" aria-label={W.eq.raise} onclick={() => place(chosen, band.x, band.gain + 0.5)}><Icon name="plus-circle" /></button>
          </div>
          {@render level(W.eq.gain, W.eq.db(band.value), band.gain, eq.ranges.gain, 0.5, (v) => place(chosen, band.x, v))}
        </div>
      {:else if eq.on && band}
        <div class="editor">
          <div class="tools">
            <button class="chip on" onclick={chooseKind}>{W.eq.kinds[band.kind]}<Icon name="nav-arrow-down" /></button>
            <span class="gap"></span>
            {#each W.eq.channels as c, ci (ci)}
              <button class="chip" class:on={band.channel === ci} onclick={() => place(chosen, band.x, band.gain, band.q, band.kind, ci)}>{c}</button>
            {/each}
            <span class="fill"></span>
            <button class="glyph quiet" aria-label={W.eq.remove} title={W.eq.remove} onclick={() => { const i = chosen; chosen = -1; void send(() => ask<string>("eq_tool", "remove", i)); }}><Icon name="trash" /></button>
          </div>
          {@render level(W.eq.frequency, band.freq, band.x, [0, 1], 0.001, (v) => place(chosen, v, band.gain))}
          {#if band.usesGain}{@render level(W.eq.gain, W.eq.db(band.value), band.gain, eq.ranges.gain, 0.5, (v) => place(chosen, band.x, v))}{/if}
          {@render level(band.kind === 8 || band.kind === 9 ? W.eq.slope : W.eq.q, band.shape, band.q, eq.ranges.q, 0.01, (v) => place(chosen, band.x, band.gain, v))}
        </div>
      {/if}
      <div class="chips-line"><button class="chip" onclick={() => { chosen = -1; void send(() => ask<string>("eq_tool", "reset", 0)); }}>{W.eq.reset}</button></div>
    </div>

    <div class="group-label">{W.eq.presets}</div>
    <div class="group plate">
      <div class="chips-line wrap">
        {#each eq.presets as p, i (p)}<button class="chip" onclick={() => void send(() => ask<string>("eq_tool", "preset", i))}>{W.eq.presetNames[p] ?? p}</button>{/each}
      </div>
      <label class="group-row">
        <span class="words"><span>{W.eq.preampAuto}</span><span class="sub">{W.eq.preampAutoNote}</span></span>
        <input type="checkbox" class="switch" aria-label={W.eq.preampAuto} checked={eq.auto} onchange={(e) => void set("eqPreampDb", e.currentTarget.checked ? "auto" : String(eq!.preamp))} />
      </label>
      {#if eq.auto}<p class="hint">{W.eq.preamp} {W.eq.db(eq.preamp.toFixed(1))}</p>
      {:else}{@render level(W.eq.preamp, W.eq.db(eq.preamp.toFixed(1)), eq.preamp, eq.ranges.preamp, 0.5, (v) => void set("eqPreampDb", String(v)))}{/if}
    </div>

    <div class="group-label">{W.eq.output}</div>
    <div class="group plate">
      {@render level(W.eq.balance, W.eq.balanceAt(eq.balance), eq.balance, eq.ranges.balance, 0.05, (v) => void set("balance", String(v)))}
      <label class="group-row"><span class="words">{W.eq.mono}</span><input type="checkbox" class="switch" aria-label={W.eq.mono} checked={eq.mono} onchange={(e) => void set("mono", e.currentTarget.checked ? "true" : "false")} /></label>
      <label class="group-row"><span class="words">{W.eq.limiter}</span><input type="checkbox" class="switch" aria-label={W.eq.limiter} checked={eq.limiter} onchange={(e) => void set("limiter", e.currentTarget.checked ? "true" : "false")} /></label>
      {#if eq.limiter}{@render level(W.eq.ceiling, W.eq.db(eq.ceiling.toFixed(1)), eq.ceiling, eq.ranges.limiter, 0.5, (v) => void set("limiterThresholdDb", String(v)))}{/if}
    </div>
  {/if}
</Page>

{#snippet level(name: string, shown: string, value: number, range: [number, number], by: number, moved: (v: number) => void)}
  <div class="level">
    <span class="name">{name}</span>
    <input
      class="range"
      type="range"
      min={range[0]}
      max={range[1]}
      step={by}
      {value}
      style:--v="{((value - range[0]) / (range[1] - range[0] || 1)) * 100}%"
      aria-label={name}
      oninput={(e) => moved(Number(e.currentTarget.value))}
    />
    <span class="value">{shown}</span>
  </div>
{/snippet}

<style>
  .plate {
    margin: 0 -4px;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .chips-line {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow-x: auto;
  }
  .chips-line.wrap {
    flex-wrap: wrap;
  }
  .gap {
    width: 12px;
    flex: none;
  }
  .fill {
    flex: 1;
  }
  .curve {
    position: relative;
    height: 240px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--ink) 5%, transparent);
    overflow: hidden;
  }
  svg {
    display: block;
    touch-action: none;
    user-select: none;
  }
  svg.adds {
    cursor: crosshair;
  }
  .grid {
    stroke: var(--hair);
    stroke-opacity: 0.35;
    stroke-width: 1;
  }
  .grid.zero {
    stroke-opacity: 0.8;
  }
  .scale {
    fill: var(--ink2);
    font-size: 10px;
  }
  .shape {
    fill: color-mix(in srgb, var(--ink) 4%, transparent);
    stroke: color-mix(in srgb, var(--ink) 12%, transparent);
  }
  .shape.lit {
    fill: color-mix(in srgb, var(--accent) 22%, transparent);
    stroke: color-mix(in srgb, var(--accent) 60%, transparent);
  }
  .area {
    fill: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .line {
    fill: none;
    stroke: var(--accent);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .off .line {
    stroke: var(--ink2);
  }
  .off .area {
    fill: color-mix(in srgb, var(--ink2) 10%, transparent);
  }
  .held {
    stroke: var(--accent);
    stroke-opacity: 0.5;
  }
  .point {
    cursor: move;
  }
  .point.upright {
    cursor: ns-resize;
  }
  .off .point {
    cursor: default;
  }
  .reach {
    fill: transparent;
  }
  .dot {
    fill: var(--ink);
    stroke: color-mix(in srgb, var(--ink) 5%, var(--surface));
    stroke-width: 2;
  }
  .off .dot {
    fill: var(--ink2);
  }
  .point:hover .dot {
    r: 7;
  }
  .point.lit .dot {
    fill: var(--accent);
  }
  .bubble {
    position: absolute;
    top: 6px;
    min-width: 110px;
    padding: 4px 8px;
    border-radius: 999px;
    background: var(--surface);
    border: var(--hairline) solid var(--hair);
    font-size: 11px;
    text-align: center;
    pointer-events: none;
  }
  .hint,
  .bypass {
    margin: 0;
    font-size: 12px;
    color: var(--ink2);
  }
  .bypass {
    padding: 10px 12px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--ink);
    margin-bottom: 12px;
  }
  .editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .level {
    display: grid;
    grid-template-columns: 90px 1fr 80px;
    align-items: center;
    gap: 12px;
    min-height: 32px;
  }
  .level .value {
    text-align: right;
    font-size: 12px;
    color: var(--ink2);
  }
  .level .range {
    width: 100%;
  }
</style>
