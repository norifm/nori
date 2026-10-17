// Making elements without a framework.

type Child = Node | string | null | undefined | false;
type Props = Record<string, unknown> & { class?: string; on?: Record<string, (e: Event) => void> };

/** `<tag>` with `props` (attributes, `class`, `on` handlers) and `children`. */
export function h<K extends keyof HTMLElementTagNameMap>(tag: K, props: Props | null = null, ...children: Child[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props ?? {})) {
    if (k === "on") for (const [ev, f] of Object.entries(v as Record<string, (e: Event) => void>)) el.addEventListener(ev, f);
    else if (k === "class") el.className = v as string;
    else if (v === true) el.setAttribute(k, "");
    else if (v !== false && v !== undefined && v !== null) el.setAttribute(k, String(v));
  }
  for (const c of children) if (c !== null && c !== undefined && c !== false) el.append(c);
  return el;
}

/** An SVG icon from the sprite in index.html. */
export function icon(name: string, label?: string): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "icon");
  svg.setAttribute("aria-hidden", label ? "false" : "true");
  if (label) svg.setAttribute("aria-label", label);
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#i-${name}`);
  svg.append(use);
  return svg;
}

/** A round icon button. */
export function iconButton(name: string, label: string, onClick: () => void, cls = "round"): HTMLButtonElement {
  const b = h("button", { class: cls, title: label, "aria-label": label, on: { click: (e) => (e.stopPropagation(), onClick()) } }, icon(name));
  return b;
}

/** A cover image, lazily loaded; a plain square while there is none. */
export function cover(src: string | undefined, cls = "cover"): HTMLElement {
  const box = h("div", { class: cls });
  if (src) box.append(h("img", { src, alt: "", loading: "lazy", decoding: "async" }));
  return box;
}
