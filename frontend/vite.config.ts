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
  },
})
