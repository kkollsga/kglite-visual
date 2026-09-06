import { expect, test } from '@playwright/test'

import { requestNonce } from '../../src/request-id'

test('request nonces use getRandomValues and remain unique UUID-shaped values', () => {
  const nonces = Array.from({length: 64}, requestNonce)
  expect(new Set(nonces).size).toBe(nonces.length)
  for (const nonce of nonces) expect(nonce).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/)
})
