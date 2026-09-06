/** Drive the actual Python repr_html iframe at notebook width. Output belongs in bench/out. */
import { spawn } from 'node:child_process'
import { createRequire } from 'node:module'
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { once } from 'node:events'
import { createInterface } from 'node:readline'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const require = createRequire(path.join(repo, 'frontend/package.json'))
const { chromium, expect } = require('@playwright/test')
const python = path.resolve(repo, process.argv[2] ?? '.venv/bin/python')
const out = path.resolve(repo, process.argv[3] ?? 'dev-docs/bench/out/notebook-workspace')
mkdirSync(out, { recursive: true })
const config = mkdtempSync(path.join(os.tmpdir(), 'kglv-notebook-check-'))
const script = `import json,sys
import kglite_visual as kv
from kglite_visual import _notebook
_notebook.remote_reason=lambda: None
_notebook.proxy_url=lambda port: None
with kv.show(sys.argv[1], open_browser=False, height=480) as view:
    print(json.dumps({'html':view._repr_html_(),'url':view.url,'port':view.port}),flush=True)
    sys.stdin.read()
`
const child = spawn(python, ['-u', '-c', script, path.join(repo, 'crates/kglite-visual-core/tests/fixtures/meta.kgl')], {
  cwd: repo, env: { ...process.env, KGLITE_VISUAL_CONFIG_DIR: config }, stdio: ['pipe', 'pipe', 'pipe'],
})
const exit = once(child, 'exit')
let stderr = ''
child.stderr.on('data', data => { stderr += data })
let browser
try {
  const info = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`Python launch timeout: ${stderr}`)), 20_000)
    createInterface({ input: child.stdout }).once('line', line => {
      clearTimeout(timer)
      try { resolve(JSON.parse(line)) } catch (error) { reject(error) }
    })
    child.once('exit', code => { clearTimeout(timer); reject(new Error(`Python launch exited ${code}: ${stderr}`)) })
  })
  if (!info.html.includes('<iframe')) throw new Error('Python repr_html produced no local iframe')
  browser = await chromium.launch({ headless: true, args: ['--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader'] })
  const page = await browser.newPage({ viewport: { width: 720, height: 780 } })
  page.on('pageerror', error => console.error(`Notebook page error: ${error.message}`))
  const viewRequestIds = []
  page.on('request', request => {
    if (!request.url().endsWith('/api/views/save')) return
    const requestId = request.postDataJSON()?.request_id
    if (typeof requestId === 'string') viewRequestIds.push(requestId)
  })
  await page.setContent(`<main style="width:640px;max-width:100%">${info.html}</main>`)
  const frame = page.frameLocator('iframe')
  await expect(frame.getByTestId('destination-explore')).toBeVisible()
  const content = await page.locator('iframe').elementHandle().then(element => element.contentFrame())
  const randomUuidType = await content.evaluate(() => typeof crypto.randomUUID)
  if (randomUuidType !== 'undefined') throw new Error(`Notebook iframe unexpectedly exposes crypto.randomUUID (${randomUuidType}); nonce fallback was not exercised`)
  await expect(frame.getByTestId('count-loaded')).toHaveText('0')
  await frame.getByTestId('destination-query').click()
  await expect(frame.getByTestId('query-editor')).toBeVisible()
  await frame.getByTestId('destination-data').click()
  await expect(frame.getByTestId('data-records')).toBeVisible()
  await frame.getByTestId('destination-explore').click()
  const response = await fetch(new URL('/api/browse-type', info.url), {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ node_type: 'Person', limit: 60 }),
  })
  if (!response.ok) throw new Error(`Embedded shared browse failed ${response.status}: ${await response.text()}`)
  await expect(frame.getByTestId('count-loaded')).toHaveText('60')
  await expect(frame.getByTestId('scope-instances')).toHaveAttribute('aria-pressed', 'true')
  await frame.getByTestId('destination-data').click()
  await expect(frame.getByTestId('records-page')).toContainText('1–60 of 60')
  await frame.getByTestId('records-table').getByRole('checkbox').first().check()
  await expect(frame.getByTestId('records-selection')).toContainText('1 selected')
  await frame.getByTestId('views-open').click()
  await expect(frame.getByTestId('view-storage-note')).not.toContainText('Checking')
  for (const name of ['notebook-nonce-a', 'notebook-nonce-b']) {
    await frame.getByTestId('view-name').fill(name)
    await frame.getByTestId('view-save').click()
    await expect(frame.getByTestId('views-status')).toContainText(`Saved “${name}”`)
  }
  if (viewRequestIds.length !== 2 || new Set(viewRequestIds).size !== 2 || viewRequestIds.some(id => !/^view-[0-9a-f-]{36}$/.test(id))) {
    throw new Error(`Saved-view request nonces were not unique UUID-shaped values: ${JSON.stringify(viewRequestIds)}`)
  }
  const width = await content.evaluate(() => ({ viewport: innerWidth, body: document.documentElement.scrollWidth }))
  if (width.body > width.viewport + 1) throw new Error(`Iframe horizontal overflow: ${JSON.stringify(width)}`)
  await page.screenshot({ path: path.join(out, 'notebook-workspace.png') })
  writeFileSync(path.join(out, 'result.json'), JSON.stringify({ source: 'actual Python View._repr_html_', simulated_environment: 'local kernel without proxy', notebook_frame_height: 480, parent_width: 640, loaded: 60, selected: 1, width, status: 'passed' }, null, 2))
  console.log(`Notebook iframe tasks passed; artifacts: ${out}`)
} finally {
  await browser?.close()
  child.stdin.end()
  const killTimer = setTimeout(() => child.kill('SIGTERM'), 10_000)
  const [code, signal] = await exit
  clearTimeout(killTimer)
  rmSync(config, { recursive: true, force: true })
  if (code !== 0) throw new Error(`Python shutdown failed ${code}/${signal}: ${stderr}`)
}
