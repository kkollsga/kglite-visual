import { defineConfig } from '@playwright/test'

/**
 * L3 end-to-end (test-plan). The suite launches the real binary itself, so
 * there is deliberately no `webServer` here: the launch contract — one JSON
 * line on stdout with the resolved port — is part of what is under test, and
 * letting Playwright start the process would hide it.
 */
export default defineConfig({
  // Both tiers under one runner: `tests/unit` is pure TypeScript with no
  // browser, `tests/e2e` launches the binary. Splitting them across two test
  // frameworks would mean two configs and two ways to be skipped.
  testDir: './tests',
  // One worker: each test spawns a server that loads a graph, and the point of
  // the suite is determinism, not throughput.
  workers: 1,
  fullyParallel: false,
  // A hang here is a renderer that never initialised — the failure this suite
  // exists to catch — so the timeout must fire rather than wait out CI.
  timeout: 60_000,
  reporter: [['list']],
  outputDir: './tests/e2e/artifacts',
  use: {
    // The renderer is WebGL2. Headless Chromium has no GPU, so it needs
    // ANGLE over SwiftShader — luma.gl's own CI flag set, which is the
    // strongest available evidence that this stack renders headlessly at all.
    launchOptions: {
      args: [
        '--use-gl=angle',
        '--use-angle=swiftshader',
        '--enable-unsafe-swiftshader',
      ],
    },
    // Fixed, because the label overlay is screen-space: a viewport that
    // changed between runs would change which labels win their cells.
    viewport: { width: 1280, height: 800 },
  },
})
