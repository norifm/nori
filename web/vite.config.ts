// The shell: the Svelte app (index.html), its workers and the service worker, as static files in
// web/dist beside the wasm modules web/build.sh put in dist/pkg. `--mode decoder` builds dist/decoder.js
// on its own: the player modules' glue imports it by that address, and the worker the same instance.
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig(({ mode }) =>
  mode === "decoder"
    ? {
        build: {
          outDir: "dist",
          emptyOutDir: false,
          target: "es2022",
          reportCompressedSize: false,
          lib: { entry: "src/decoder.ts", formats: ["es"], fileName: () => "decoder.js" },
        },
      }
    : {
        base: "./",
        plugins: [svelte()],
        build: {
          outDir: "dist",
          // dist/pkg holds the wasm modules, built before.
          emptyOutDir: false,
          target: "es2022",
          modulePreload: { polyfill: false },
          reportCompressedSize: false,
          rollupOptions: {
            input: { index: "index.html", sw: "src/sw.ts" },
            // The service worker keeps one address: a new build is found at it.
            output: { entryFileNames: (c) => (c.name === "sw" ? "sw.js" : "assets/[name]-[hash].js") },
          },
        },
        worker: { format: "es" },
        // The top bar's wordmark is docs/brand's.
        server: { fs: { allow: [".", "../docs/brand"] } },
      },
);
