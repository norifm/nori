// The service worker: the shell and the wasm modules of one build, kept so a reload or a start without
// the network is instant. Never the API or audio (no offline music in v1). web/build.sh fills in the
// build's hash and files. A new build installs beside the running one and takes over at the next load
// (the page asks it to, before anything starts).

const BUILD = "__BUILD__";
const FILES: string[] = JSON.parse('__FILES__');
const CACHE = `nori-${BUILD}`;

type Lifecycle = { waitUntil(p: Promise<unknown>): void };
type Fetching = Lifecycle & { request: Request; respondWith(r: Promise<Response>): void };
const sw = self as unknown as {
  addEventListener(type: "install" | "activate", f: (e: Lifecycle) => void): void;
  addEventListener(type: "fetch", f: (e: Fetching) => void): void;
  addEventListener(type: "message", f: (e: { data: unknown }) => void): void;
  skipWaiting(): Promise<void>;
  registration: { scope: string };
};

sw.addEventListener("install", (e) => e.waitUntil(caches.open(CACHE).then((c) => c.addAll(FILES))));

sw.addEventListener("activate", (e) =>
  e.waitUntil(caches.keys().then((keys) => Promise.all(keys.filter((k) => k.startsWith("nori-") && k !== CACHE).map((k) => caches.delete(k))))),
);

// The page found this build waiting at its start: it takes over before the page loads again.
sw.addEventListener("message", (e) => {
  if (e.data === "take over") void sw.skipWaiting();
});

sw.addEventListener("fetch", (e) => {
  const url = new URL(e.request.url);
  const path = url.pathname.slice(new URL(sw.registration.scope).pathname.length);
  // The server's API and songs go to the network as they are.
  if (e.request.method !== "GET" || url.origin !== location.origin || path.startsWith("rest/") || path.startsWith("nori/")) return;
  const file = path === "" || e.request.mode === "navigate" ? "index.html" : path;
  if (!FILES.includes(file) && !file.startsWith("pkg/")) return;
  e.respondWith(
    caches.open(CACHE).then(async (c) => {
      const kept = await c.match(file);
      if (kept) return kept;
      // A module of this build not kept at install (the Rust-codecs player): kept once fetched.
      const got = await fetch(e.request);
      if (got.ok) await c.put(file, got.clone());
      return got;
    }),
  );
});
