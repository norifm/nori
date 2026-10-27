// A list whose rows ([data-row], each with its place in `data-at`) are dragged to another place, by mouse
// or touch, as Android's queue: the row held follows the pointer, the rows it passes step out of its way,
// and on letting go it glides into its slot and `moved(from, to)` is asked. A row with a handle
// ([data-handle]) is taken by it at once; with `whole`, a row is taken anywhere but on its own buttons
// ([data-still]): by a mouse once it moves a little (a click still plays), by a finger once it has rested
// a moment (a swipe still scrolls). The rows keep where they were drawn until the list has changed, so
// nothing slides back first; while they land the list carries `landing`, for its own animation to wait.

const STEP_MS = 160;
const DROP_MS = 120;
/** A move that never changes the list does not hold the rows up for good. */
const LANDING_MS = 1000;
/** How far a mouse moves a row before it is a drag, not a click, px. */
const MOUSE_SLOP = 4;
/** How long a finger rests on a row to take it, ms, and how far it may wander meanwhile, px. */
const PRESS_MS = 350;
const PRESS_SLOP = 8;

type Options = { moved: (from: number, to: number) => unknown; whole?: boolean };

export function sortable(list: HTMLElement, options: Options) {
  let opts = options;
  let held: { row: HTMLElement; rows: HTMLElement[]; rects: DOMRect[]; at: number; to: number; y: number } | null = null;
  /** A row pressed, not taken yet. */
  let pressed: { row: HTMLElement; id: number; x: number; y: number; touch: boolean; timer?: ReturnType<typeof setTimeout> } | null = null;
  /** A drag just ended: the click it ends in is not a click on the row. */
  let dragged = false;
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
    list.classList.remove("landing");
  };

  function take(row: HTMLElement, pointer: number, y: number) {
    const rows = [...list.querySelectorAll<HTMLElement>("[data-row]")];
    const at = rows.indexOf(row);
    held = { row, rows, rects: rows.map((r) => r.getBoundingClientRect()), at, to: at, y };
    row.setPointerCapture(pointer);
    row.classList.add("held");
    dragged = true;
  }
  const letGo = () => {
    if (pressed?.timer) clearTimeout(pressed.timer);
    pressed = null;
  };

  const down = (e: PointerEvent) => {
    if (e.button !== 0 || held) return;
    const target = e.target as HTMLElement;
    const row = target.closest<HTMLElement>("[data-row]");
    if (!row) return;
    dragged = false;
    if (target.closest("[data-handle]")) {
      e.preventDefault();
      take(row, e.pointerId, e.clientY);
      return;
    }
    if (!opts.whole || target.closest("[data-still]")) return;
    const touch = e.pointerType !== "mouse";
    const p: NonNullable<typeof pressed> = { row, id: e.pointerId, x: e.clientX, y: e.clientY, touch };
    if (touch)
      p.timer = setTimeout(() => {
        pressed = null;
        take(p.row, p.id, p.y);
        navigator.vibrate?.(10);
      }, PRESS_MS);
    pressed = p;
  };

  const move = (e: PointerEvent) => {
    if (pressed) {
      const far = Math.hypot(e.clientX - pressed.x, e.clientY - pressed.y);
      if (pressed.touch && far > PRESS_SLOP) letGo();
      else if (!pressed.touch && far > MOUSE_SLOP) {
        const { row, id, y } = pressed;
        letGo();
        take(row, id, y);
      }
    }
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
    letGo();
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
      list.classList.add("landing");
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

  // A finger holding a row moves it, not the page; a press held to take a row opens no menu.
  const touchMove = (e: TouchEvent) => {
    if (held) e.preventDefault();
  };
  const menu = (e: Event) => {
    if (held || pressed?.touch) {
      e.preventDefault();
      e.stopPropagation();
    }
  };
  const click = (e: Event) => {
    if (!dragged) return;
    e.preventDefault();
    e.stopPropagation();
    dragged = false;
  };

  list.addEventListener("pointerdown", down);
  list.addEventListener("pointermove", move);
  list.addEventListener("pointerup", up);
  list.addEventListener("pointercancel", up);
  list.addEventListener("touchmove", touchMove, { passive: false });
  list.addEventListener("contextmenu", menu, true);
  list.addEventListener("click", click, true);
  return {
    update(next: Options) {
      opts = next;
    },
    destroy() {
      letGo();
      list.removeEventListener("pointerdown", down);
      list.removeEventListener("pointermove", move);
      list.removeEventListener("pointerup", up);
      list.removeEventListener("pointercancel", up);
      list.removeEventListener("touchmove", touchMove);
      list.removeEventListener("contextmenu", menu, true);
      list.removeEventListener("click", click, true);
    },
  };
}
