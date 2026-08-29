/**
 * A minimal MCP client over streamable HTTP, for the e2e suite.
 *
 * **Hand-written on purpose.** The alternative is pulling an MCP SDK into the
 * frontend's dependency tree to test a Rust server, which would make the
 * frontend's `npm audit` surface and its lockfile answer for a backend
 * protocol. More to the point, an SDK would hide the thing under test: this
 * file speaks the transport literally — POST JSON-RPC, read back either
 * `application/json` or an SSE stream, carry `mcp-session-id` — so a spec that
 * passes here is evidence the wire works, not evidence that two copies of the
 * same SDK agree.
 *
 * It implements exactly what the suite needs: `initialize`, the `initialized`
 * notification, `tools/list`, `tools/call`. Nothing about sampling, roots,
 * cancellation or resumption, none of which this server offers.
 */

/** One JSON-RPC response, as the server sent it. */
type RpcResponse = {
  result?: Record<string, unknown>
  error?: { code: number; message: string }
}

/** A tool's answer: the content blocks, plus whether it failed. */
export type ToolResult = {
  isError: boolean
  /** Concatenated text blocks — every tool here answers with JSON in one. */
  text: string
  /** Parsed from {@link text}. Throws if the tool answered with prose. */
  json: <T = Record<string, unknown>>() => T
  /** Base64 image blocks, in order, with their mime types. */
  images: { mimeType: string; base64: string }[]
}

export type Tool = {
  name: string
  description?: string
  inputSchema: { type?: string; properties?: Record<string, unknown> }
}

/**
 * Pull the JSON-RPC payload out of a response that may be either shape.
 *
 * rmcp answers a simple request/response call as `text/event-stream` by
 * default, so "read the body as JSON" is wrong exactly often enough to be
 * confusing. The SSE framing here is the minimum: `data:` lines, the last one
 * carrying the reply.
 */
async function readRpc(response: Response): Promise<RpcResponse | null> {
  const body = await response.text()
  const type = response.headers.get('content-type') ?? ''
  if (type.includes('application/json')) return JSON.parse(body) as RpcResponse
  if (!type.includes('text/event-stream')) return null
  const payloads = body
    .split('\n')
    .filter((line) => line.startsWith('data: '))
    .map((line) => line.slice('data: '.length))
    .filter((line) => line.startsWith('{'))
  const last = payloads.at(-1)
  return last === undefined ? null : (JSON.parse(last) as RpcResponse)
}

export class McpClient {
  private sessionId: string | null = null
  private nextId = 1

  constructor(private readonly url: string) {}

  private async post(body: Record<string, unknown>): Promise<Response> {
    const headers: Record<string, string> = {
      'content-type': 'application/json',
      // Both, because the server chooses: a single response may come back as
      // JSON, and anything that streams comes back as SSE.
      accept: 'application/json, text/event-stream',
    }
    if (this.sessionId !== null) headers['mcp-session-id'] = this.sessionId
    return fetch(this.url, { method: 'POST', headers, body: JSON.stringify(body) })
  }

  private async request(method: string, params?: Record<string, unknown>): Promise<RpcResponse> {
    const response = await this.post({
      jsonrpc: '2.0',
      id: this.nextId++,
      method,
      ...(params === undefined ? {} : { params }),
    })
    if (!response.ok) {
      throw new Error(`${method} -> HTTP ${response.status}: ${await response.text()}`)
    }
    const session = response.headers.get('mcp-session-id')
    if (session !== null) this.sessionId = session
    const rpc = await readRpc(response)
    if (rpc === null) throw new Error(`${method} -> no JSON-RPC payload in the response`)
    if (rpc.error !== undefined) {
      throw new Error(`${method} -> JSON-RPC error ${rpc.error.code}: ${rpc.error.message}`)
    }
    return rpc
  }

  /** The handshake. Returns the server's own `InitializeResult`. */
  async initialize(): Promise<{
    protocolVersion: string
    serverInfo: { name: string; version: string }
    instructions?: string
    capabilities: Record<string, unknown>
  }> {
    const rpc = await this.request('initialize', {
      protocolVersion: '2025-06-18',
      capabilities: {},
      clientInfo: { name: 'kglite-visual-e2e', version: '0' },
    })
    // A notification, so there is no id and no reply to wait for — but it has
    // to be sent, or a server that gates on the lifecycle refuses everything
    // after it.
    await this.post({ jsonrpc: '2.0', method: 'notifications/initialized' })
    return rpc.result as never
  }

  async listTools(): Promise<Tool[]> {
    const rpc = await this.request('tools/list')
    return (rpc.result as { tools: Tool[] }).tools
  }

  async call(name: string, args: Record<string, unknown> = {}): Promise<ToolResult> {
    const rpc = await this.request('tools/call', { name, arguments: args })
    const result = rpc.result as {
      isError?: boolean
      content?: { type: string; text?: string; data?: string; mimeType?: string }[]
    }
    const content = result.content ?? []
    const text = content
      .filter((block) => block.type === 'text')
      .map((block) => block.text ?? '')
      .join('')
    return {
      isError: result.isError === true,
      text,
      json: <T,>() => JSON.parse(text) as T,
      images: content
        .filter((block) => block.type === 'image')
        .map((block) => ({ mimeType: block.mimeType ?? '', base64: block.data ?? '' })),
    }
  }
}
