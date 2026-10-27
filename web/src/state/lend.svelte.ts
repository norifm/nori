// A page about one cover lends the page and the chrome its colors (`ui.tint`) while it is open, the
// season's until they come, and gets the cover's look (its wash) to draw. Called while a component is made.
import { coverLook, type CoverLook } from "./coverLook";
import { looks } from "./theme.svelte";
import { ui } from "./ui.svelte";

/** Lends the colors of the cover at `cover()` while the calling component lives; its look, once read. */
export function lendCover(cover: () => string | undefined): { readonly look: CoverLook | null } {
  const me = {};
  let look = $state.raw<CoverLook | null>(null);
  $effect(() => {
    void looks.rev;
    const src = cover();
    let gone = false;
    if (src)
      void coverLook(src).then((l) => {
        if (gone) return;
        ui.tint = l ? { by: me, vars: l.vars } : null;
        look = l;
      });
    else {
      ui.tint = null;
      look = null;
    }
    return () => {
      gone = true;
    };
  });
  $effect(() => () => {
    if (ui.tint?.by === me) ui.tint = null;
  });
  return {
    get look() {
      return look;
    },
  };
}
