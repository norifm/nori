// A list whose rows ([data-row], each with its place in `data-at`) are dragged by their handle
// ([data-handle]) to another place, by mouse or touch, as Android's queue: the row held follows the
// pointer, the rows it passes step out of its way, and on letting go it glides into its slot and
// `moved(from, to)` is asked. The rows keep where they were drawn until the list has changed, so nothing
// slides back first.

const STEP_MS = 160;
const DROP_MS = 120;
/** A move that never changes the list does not hold the rows up for good. */
const LANDING_MS = 1000;

type Options = { moved: (from: number, to: number) => unknown };

export function sortable(list: HTMLElement, options: Options) {
  let opts = options;
  let held: { row: HTMLElement; rows: HTMLElement[]; rects: DOMRect[]; at: number; to: number; y: number } | null = null;
  const reduced = () => document.documentElement.classList.contains("still");

  const place = (el: HTMLElement, dy: number, ms: number) => {
    el.style.transition = ms && !reduced() ? `transform ${ms}ms ease-out` : "none";
    el.style.transform = dy ? `translateY(${dy}px)` : "";
  };
  const reset = (rows: HTMLElement[]) => {
    for (const r of rows) {
      place(r, 0, 0);
      r.classList.remove("held");
    }
  };

  const down = (e: PointerEvent) => {
    const handle = (e.target as HTMLElement).closest<HTMLElement>("[data-handle]");
    const row = handle?.closest<HTMLElement>("[data-row]");
    if (!handle || !row || e.button !== 0 || held) return;
    e.preventDefault();
    const rows = [...list.querySelectorAll<HTMLElement>("[data-row]")];
    const at = rows.indexOf(row);
    held = { row, rows, rects: rows.map((r) => r.getBoundingClientRect()), at, to: at, y: e.clientY };
    handle.setPointerCapture(e.pointerId);
    row.classList.add("held");
  };

  const move = (e: PointerEvent) => {
    if (!held) return;
    const { row, rows, rects, at } = held;
    const dy = e.clientY - held.y;
    const mid = rects[at].top + rects[at].height / 2 + dy;
    let to = at;
    rects.forEach((r, i) => {
      const passed = i > at ? mid > r.top + r.height / 2 : i < at && mid < r.top + r.height / 2;
      if (passed && (i > at ? i > to : i < to)) to = i;
    });
    held.to = to;
    place(row, dy, 0);
    const tall = rects[at].height;
    rows.forEach((r, i) => i !== at && place(r, i > at && i <= to ? -tall : i < at && i >= to ? tall : 0, STEP_MS));
  };

  const up = () => {
    if (!held) return;
    const { row, rows, rects, at, to } = held;
    held = null;
    if (to === at) {
      place(row, 0, STEP_MS);
      row.classList.remove("held");
      return;
    }
    // Its slot: where the row it lands on starts, or ends when it went down.
    const slot = to > at ? rects[to].bottom - rects[at].bottom : rects[to].top - rects[at].top;
    place(row, slot, DROP_MS);
    setTimeout(() => {
      const watch = new MutationObserver(() => {
        watch.disconnect();
        reset(rows);
      });
      watch.observe(list, { childList: true, subtree: true });
      setTimeout(() => {
        watch.disconnect();
        reset(rows);
      }, LANDING_MS);
      void opts.moved(Number(rows[at].dataset.at), Number(rows[to].dataset.at));
    }, reduced() ? 0 : DROP_MS);
  };

  list.addEventListener("pointerdown", down);
  list.addEventListener("pointermove", move);
  list.addEventListener("pointerup", up);
  list.addEventListener("pointercancel", up);
  return {
    update(next: Options) {
      opts = next;
    },
    destroy() {
      list.removeEventListener("pointerdown", down);
      list.removeEventListener("pointermove", move);
      list.removeEventListener("pointerup", up);
      list.removeEventListener("pointercancel", up);
    },
  };
}
