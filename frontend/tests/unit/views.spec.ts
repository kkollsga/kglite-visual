import { expect, test } from '@playwright/test'
import { selectionKey, saveMessage } from '../../src/views'
import type { ViewReference } from '../../src/generated/ViewReference'

test('saved selection compares identities independently of order without delimiter collisions', () => {
  const node: ViewReference = {kind: 'node', handle: {generation: 'g', node_id: 1}}
  const type: ViewReference = {kind: 'type', name: 'A'}
  expect(selectionKey([node, type, node])).toBe(selectionKey([type, node]))
  expect(selectionKey([type, {kind: 'type', name: 'B'}])).not.toBe(selectionKey([{kind: 'type', name: 'A\ntype:B'}]))
  expect(selectionKey([node])).not.toBe(selectionKey([{kind: 'node', handle: {generation: 'other', node_id: 1}}]))
})

test('a saved file with a structured marker conflict stays saved and has a human status', () => {
  const result = {saved: {name: 'Example', storage: 'durable' as const, saved_at: 0}, marker_applied: false, dirty: true, marker_error: {message: 'revision changed'}}
  expect(saveMessage(result, false)).toContain('Its saved copy is intact')
  expect(saveMessage(result, false)).not.toContain('[object Object]')
  expect(saveMessage({...result, marker_applied: true, dirty: false}, true)).toContain('current changes remain unsaved')
})
