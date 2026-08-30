/**
 * "Is this query wrong?" asked of the engine, without running it.
 *
 * **Plain HTTP, like the saved-query store and for the same reason** (plan E3).
 * The answer is a handful of strings, it does not move the slot space, and
 * putting it on the wire that carries typed arrays would mean a message type, a
 * protocol bump and a decoder for three fields. It is also the only request
 * this app makes *while the user is typing*, which is another reason to keep it
 * off the socket feeding the renderer.
 *
 * **The server is the only opinion about validity.** There is no client-side
 * grammar here and there deliberately is not one: kglite's parser answers with
 * a line, a column and the token it expected, and a second grammar in the
 * browser would be a weaker opinion that disagrees with the engine on exactly
 * the inputs a user needs help with.
 */

import type { Diagnostic } from './generated/Diagnostic'
import type { ValidateResponse } from './generated/ValidateResponse'
import { apiUrl } from './urls'

/**
 * Ask the server what is wrong with `query`.
 *
 * A network failure answers `[]` rather than throwing. The editor is the caller
 * and it is asking a background question: turning "the server did not answer"
 * into a red underline would be claiming the query is invalid on the evidence
 * that the connection dropped.
 */
export async function validateQuery(query: string): Promise<Diagnostic[]> {
  try {
    const response = await fetch(apiUrl('api/validate'), {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ query }),
    })
    if (!response.ok) return []
    return ((await response.json()) as ValidateResponse).diagnostics
  } catch {
    return []
  }
}
