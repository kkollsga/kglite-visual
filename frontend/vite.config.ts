import { defineConfig } from 'vite'

export default defineConfig({
  resolve: {
    alias: {
      // Upstream packaging bug, worked around here rather than upstream:
      // gl-bench 1.0.42 (a hard dependency of @cosmos.gl/graph, which does
      // `import GLBench from 'gl-bench'`) points its `browser` field at
      // dist/gl-bench.min.js, an IIFE that assigns a global and exports
      // nothing. Vite resolves `browser` ahead of `module` for the browser
      // target, so the production build fails with
      // `"default" is not exported by gl-bench.min.js`. Its `module` field
      // build IS proper ESM with a default export. Aliasing straight to it is
      // narrower than reordering resolve.mainFields globally, which would
      // change resolution for every other dependency too.
      //
      // Delete this the day gl-bench ships a correct `browser` build — and
      // check by removing it, not by reading a changelog.
      'gl-bench': 'gl-bench/dist/gl-bench.module.js',
    },
  },
  // Relative, not '/'. The bundle is served three ways — the CLI's embedded
  // static handler, the Python wheel, and jupyter-server-proxy under a path
  // prefix like /proxy/8731/. An absolute base breaks the third silently: the
  // page loads and every asset 404s. Getting this right at birth is free;
  // retrofitting it means auditing every URL in the app (plan D7).
  base: './',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // rust-embed bakes whatever is in dist/ into the binary. A stale bundle
    // inside a fresh binary reads exactly like a backend bug, so the build
    // must not leave last week's chunks lying beside this week's.
    sourcemap: true,
    // 700 KB raw, and the number is a ratchet rather than a silencer.
    //
    // The default 500 has been firing since the renderer landed, so the choice
    // was between splitting the entry chunk and moving the line. Measured on
    // 2026-08-30 by summing the entry chunk's own source map by package: of
    // 1355.6 KB pre-minify, 410.2 KB is @cosmos.gl/graph, 428.1 KB is the four
    // @luma.gl packages, 126.1 KB is dompurify, and the app's own `src/` is
    // 130.7 KB — 9.6%. There is nothing in the 90% to defer: the canvas *is*
    // the entry screen, so a dynamically imported renderer would be a
    // dynamically imported first paint. The warning was reporting a fact about
    // this app's dependencies, not about anything a split could improve, and a
    // warning that fires on every build of a correct tree is one nobody reads.
    //
    // 700 leaves ~78 KB of raw headroom over today's 622.2 KB, and the one
    // large addition this ceiling was sized for has now landed inside it: the
    // CodeMirror 6 editor is `editor-*.js`, 257.3 KB raw / 84.2 KB gzipped,
    // fetched by the dynamic `import()` in `panels.ts`. The entry chunk moved
    // 621.3 → 622.2 KB for it — the swap logic, not the editor. Code splitting
    // is not theoretical here: this build also emits `webgl-device-*.js`
    // (101 KB) from a dynamic import inside luma.gl, and both are fetched at
    // runtime through the relative `base` above and served by the embedded
    // static handler, which is what the e2e suite exercises on every run.
    //
    // Crossing 700 means the entry chunk grew by something that is not the
    // renderer. Raise it again only with a fresh per-package measurement and a
    // reason, or split what grew — never because a build went yellow.
    chunkSizeWarningLimit: 700,
  },
})
