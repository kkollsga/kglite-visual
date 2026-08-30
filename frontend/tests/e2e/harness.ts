/**
 * Launching the real binary, shared by every e2e spec.
 *
 * Extracted when the drill-in spec landed beside the smoke spec: two copies of
 * a launch contract is two places for a contract change to be half-applied, and
 * this one is the thing under test in the smoke spec.
 */

import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { execFileSync } from 'node:child_process'
import { existsSync, mkdtempSync, rmSync } from 'node:fs'
import os from 'node:os'
import { createInterface } from 'node:readline'
import path from 'node:path'

import type { Page } from '@playwright/test'

// Playwright runs with the config's directory as cwd, so the repo root is one
// level up. Derived rather than hard-coded, and verified below: a wrong root
// would otherwise surface as "binary not found", which reads like a build
// problem rather than a path problem.
export const REPO = path.resolve(process.cwd(), '..')
export const FIXTURE = 'crates/kglite-visual-core/tests/fixtures/meta.kgl'

export type LaunchInfo = {
  url: string
  port: number
  pid: number
  graph: string
  /** Streamable-HTTP MCP, on the same port (plan D14). */
  mcp: string
}

export type Launched = {
  process: ChildProcessWithoutNullStreams
  info: LaunchInfo
  stderr: string[]
}

/**
 * The app URL in D2's deterministic layout mode.
 *
 * **Every spec that asserts a position, a count or a screenshot goes through
 * this**, and the reason is the thing the flag switches off. The user default
 * is the GPU force simulation: it moves every point continuously until it
 * settles, so `positionsHash` would hash vendor float behaviour and a label
 * count would depend on when the frame was taken. `?deterministic=1` restores
 * exactly the constructor this suite was written against — server-supplied
 * positions, `enableSimulation: false` — so the assertions below prove the
 * same property they always did, about the same code path the server still
 * feeds. Specs assert `layoutMode` too, so a dropped flag fails here rather
 * than being absorbed as flake.
 */
export function appUrl(info: LaunchInfo): string {
  return `${info.url}?deterministic=1`
}

/**
 * Resolve the binary through the newest-of-profile check.
 *
 * Never `target/debug/...` hard-coded, and never "release if present": the
 * script refuses a binary older than the bundle it should embed, which is the
 * exact failure this suite would otherwise report as a frontend bug.
 */
export function resolveBinary(): string {
  return execFileSync(
    'python3',
    [path.join(REPO, 'scripts/check_bundle.py'), '--resolve-binary', 'kglite-visual'],
    { encoding: 'utf8', cwd: REPO },
  ).trim()
}

/**
 * A throwaway saved-query store for the whole run.
 *
 * The server writes saved queries and recent history under the *developer's*
 * config directory by default. A suite that ran there would read whatever the
 * machine happened to hold and leave its own rows behind — a test with a side
 * effect on the person running it, and assertions that pass or fail depending
 * on what they saved last week. The env override exists for exactly this.
 *
 * In the OS temp directory rather than under the repo, so it needs no
 * accumulation bound of its own, and removed when this process exits.
 */
const QUERY_STORE = mkdtempSync(path.join(os.tmpdir(), 'kglv-e2e-queries-'))
process.on('exit', () => rmSync(QUERY_STORE, { recursive: true, force: true }))

export async function launch(): Promise<Launched> {
  if (!existsSync(path.join(REPO, FIXTURE))) {
    throw new Error(`fixture ${FIXTURE} not found under ${REPO}`)
  }
  const child = spawn(resolveBinary(), [FIXTURE, '--no-open', '--port', '0'], {
    cwd: REPO,
    env: { ...process.env, KGLITE_VISUAL_CONFIG_DIR: QUERY_STORE },
  })
  const stderr: string[] = []
  createInterface({ input: child.stderr }).on('line', (line) => stderr.push(line))

  const info = await new Promise<LaunchInfo>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`no stdout JSON line in 20s; stderr:\n${stderr.join('\n')}`)),
      20_000,
    )
    createInterface({ input: child.stdout }).on('line', (line) => {
      clearTimeout(timer)
      // The launch contract: stdout carries exactly ONE line, and it parses.
      // Scraping free-form logs or racing a hard-coded port is what this
      // replaces.
      resolve(JSON.parse(line) as LaunchInfo)
    })
    child.on('exit', (code) =>
      reject(new Error(`binary exited with ${code}; stderr:\n${stderr.join('\n')}`)),
    )
  })

  return { process: child, info, stderr }
}

/**
 * Put text in the query editor, whichever editor is live.
 *
 * The panel ships a `<textarea>` and upgrades it to CodeMirror when that chunk
 * lands, so a spec that only knew how to drive one of them would be asserting
 * on an app half the users may not have. `fill()` covers both: Playwright
 * clears and inserts through the same path for a textarea and for a
 * `contenteditable`, and CodeMirror turns that into a document change.
 *
 * The first wait is the race guard, and it waits for the *upgrade to settle*
 * rather than for a box to exist. Waiting for "a textarea or a CodeMirror"
 * would be satisfied instantly by the textarea and the spec would type into an
 * element the import is about to replace — text the editor never receives,
 * surfacing as an empty query and reading as a server bug. The panel settles
 * exactly two ways: CodeMirror mounted, or the note saying it did not.
 */
export async function fillQuery(page: Page, text: string): Promise<void> {
  await page
    .locator('[data-testid="query-editor"] .cm-content, [data-testid="editor-note"].kglv-warn')
    .first()
    .waitFor({ state: 'attached' })
  const target = page.locator(
    '[data-testid="query-editor"] .cm-content, [data-testid="query-editor"] textarea',
  )
  await target.first().fill(text)
}

/**
 * The editor's text.
 *
 * CodeMirror renders one `.cm-line` per line, so the joins are the newlines;
 * `textContent` alone would run them together and an assertion on a two-line
 * query would compare the wrong string.
 */
export async function queryText(page: Page): Promise<string> {
  const lines = page.locator('[data-testid="query-editor"] .cm-line')
  if ((await lines.count()) > 0) {
    return lines.evaluateAll((nodes) => nodes.map((node) => node.textContent ?? '').join('\n'))
  }
  return page.locator('[data-testid="query-editor"] textarea').inputValue()
}
