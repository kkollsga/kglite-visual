import { expect, test } from '@playwright/test'

import { captureQueryProvenance } from '../../src/charts/provenance'

test('query provenance keeps the exact request id, text and detached nested parameters', () => {
  const params: Record<string, unknown> = {fields: ['A', 'B'], window: {start: 2020, end: 2024}}
  const captured = captureQueryProvenance('browser-7', 'RETURN $fields, $window', params, 1234)
  ;(params.fields as string[])[0] = 'changed'
  ;(params.window as {start: number}).start = 1990

  expect(captured).toEqual({
    requestId: 'browser-7', query: 'RETURN $fields, $window',
    params: {fields: ['A', 'B'], window: {start: 2020, end: 2024}}, requestedAtMs: 1234,
  })
})
