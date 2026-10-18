// The library as the pages read it: each page's read, done again on every visit and after every change
// (a playlist edited, a heart, another profile opened); the changes themselves are the core's.

import { tick } from "svelte";
import { ask, facts, read } from "../rpc";
import { route } from "./route.svelte";
import type { Playlist } from "./types";

class Library {
  /** Bumped by every change: the pages read themselves again. */
  rev = $state(0);
  /** The sidebar's playlists. */
  playlists = $state.raw<Playlist[]>([]);

  /** Asks the server for a change (`{ kind, ... }`, crates/web pages.rs `Change`). */
  async change(c: Record<string, unknown>) {
    await ask("change", JSON.stringify(c));
    this.rev++;
  }

  /** The profile changed under the pages (a jam joined or left). */
  changed() {
    this.rev++;
  }

  async readPlaylists() {
    this.playlists = (await read<{ playlists: Playlist[] }>({ kind: "playlists" })).playlists ?? [];
  }
}

export const library = new Library();

let firstDrawn: () => void;
/** The first page drew: the player module loads after it. */
export const drawn = new Promise<void>((r) => (firstDrawn = r));

/** Page `top` drew, with what it read or why it could not: for a check driving the page. */
async function shown(top: string, error?: string) {
  await tick();
  firstDrawn();
  facts.page = top;
  if (error) facts.pageError = error;
  else delete facts.pageError;
}

/** A page's read (`reader` names what it reads from its props, read inside it): `data` once in, read
 * again on every visit and change. Called while a component starts. */
export function reading<T>(top: string, reader: () => Promise<T>): { readonly data: T | undefined; readonly error: string | undefined } {
  let data = $state.raw<T>();
  let error = $state<string>();
  $effect(() => {
    void route.visit;
    void library.rev;
    let gone = false;
    reader().then(
      (d) => {
        if (gone) return;
        data = d;
        error = undefined;
        void shown(top);
      },
      (e) => {
        if (gone) return;
        error = String((e as Error)?.message ?? e);
        void shown(top, error);
      },
    );
    return () => {
      gone = true;
    };
  });
  return {
    get data() {
      return data;
    },
    get error() {
      return error;
    },
  };
}
