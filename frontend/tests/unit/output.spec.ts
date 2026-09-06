import { expect, test } from '@playwright/test'
import { tableCsv } from '../../src/table-csv'
import { boundedBlob, downloadFilename } from '../../src/download'
import { AppearanceMappingIndex } from '../../src/mapping'

test('table CSV quotes typed values, integer precision and partial cells while refusing a whole oversized output', async () => {
  const rows = [[{state: 'value' as const, value: {type: 'int64' as const, value: '9007199254740993'}}, {state: 'value' as const, value: {type: 'string' as const, value: '9007199254740993'}}, {state: 'missing' as const}]]
  const text = await tableCsv(['integer', 'string', 'absent'], rows).text()
  expect(text).toContain('"9007199254740993","value","int64",""')
  expect(text).toContain('"""9007199254740993""","value","string",""')
  expect(text).toContain('"missing","missing","",""')
  expect(() => tableCsv(['integer', 'string', 'absent'], rows, 4)).toThrow('CSV exceeds')
})

test('bounded downloads retain UTF-8 filenames and reject oversized bodies', async () => {
  expect(downloadFilename('attachment; filename="fallback.svg"; filename*=UTF-8\'\'bl%C3%A5.svg', 'graph.svg')).toBe('blå.svg')
  expect(await (await boundedBlob(new Response('abc'))).text()).toBe('abc')
  await expect(boundedBlob(new Response(new Uint8Array(16 * 1024 * 1024 + 1)))).rejects.toThrow('16 MiB')
})

test('canonical mapping joins generation handles and preserves exact published channels with structural nulls', () => {
  const index = new AppearanceMappingIndex()
  const handle = {generation: 'g', node_id: 6}
  index.set({scope: 'loaded', nodes: [{handle, color: [0.1, 0.2, 0.3, 1], radius: 17, color_state: 'value', size_state: 'value'}], categories: [], other_categories: 0, size_min: null, size_max: null})
  expect(index.get({...handle})).toMatchObject({color: [0.1, 0.2, 0.3, 1], radius: 17})
  expect(index.get({generation: 'old', node_id: 6})).toBeUndefined()
  index.set({scope: 'loaded', nodes: [{handle, color: null, radius: null, color_state: null, size_state: null}], categories: [], other_categories: 0, size_min: null, size_max: null})
  expect(index.get(handle)?.color).toBeNull()
  expect(index.get(handle)?.radius).toBeNull()
})

test('CSV state metadata separates literal null/missing strings and preserves partial/unavailable explanations', async () => {
  const csv = await tableCsv(['x', 'x [cell state]'], [
    [{state: 'null'}, {state: 'value', value: {type: 'string', value: 'null'}}],
    [{state: 'missing'}, {state: 'value', value: {type: 'string', value: 'missing'}}],
    [{state: 'truncated', preview: 'long', reason: 'cell budget'}, {state: 'unavailable', reason: 'outside input'}],
  ]).text()
  expect(csv).toContain('"x [cell state] (2)"')
  expect(csv).toContain('"null","null","",""')
  expect(csv).toContain('"""null""","value","string",""')
  expect(csv).toContain('"missing","value","string",""')
  expect(csv).toContain('"long… [partial: cell budget]","truncated","","cell budget"')
  expect(csv).toContain('"unavailable: outside input","unavailable","","outside input"')
})
