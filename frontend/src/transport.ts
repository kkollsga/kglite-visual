/**
 * The transport seam (plan D8).
 *
 * The app talks to this interface, never to a WebSocket. P5 decides the
 * notebook transport from a spike — anywidget comm buffers if binary buffers
 * carry typed arrays acceptably, an iframe otherwise — and because the
 * protocol is message-oriented rather than a byte stream, that decision is a
 * mount-time choice here, not a fork of the client.
 */

import { decodeFrame, type Frame } from './protocol'
import { wsUrl } from './urls'

export type TransportHandlers = {
  onFrame: (frame: Frame) => void
  onStatus: (connected: boolean) => void
  onError: (message: string) => void
}

export interface Transport {
  connect(handlers: TransportHandlers): void
  /** A request. P2's server answers every one with an error frame. */
  send(request: string): void
  close(): void
}

export class WebSocketTransport implements Transport {
  private socket: WebSocket | null = null

  /**
   * @param path relative to the document, never absolute — the same bundle is
   *   served at `/` by the CLI and under a prefix like `/proxy/8731/` by
   *   jupyter-server-proxy, where an absolute path connects to the wrong
   *   server (or to nothing) while the page itself loads fine.
   */
  constructor(private readonly path = 'ws') {}

  connect(handlers: TransportHandlers): void {
    const socket = new WebSocket(wsUrl(this.path))
    // Without this the browser delivers Blobs and every frame needs an async
    // read before its header can be looked at.
    socket.binaryType = 'arraybuffer'
    this.socket = socket

    socket.onopen = () => handlers.onStatus(true)
    socket.onclose = () => handlers.onStatus(false)
    socket.onerror = () =>
      handlers.onError(`websocket failed to connect to ${wsUrl(this.path)}`)
    socket.onmessage = (event: MessageEvent<unknown>) => {
      if (!(event.data instanceof ArrayBuffer)) {
        // The server sends only binary frames. A text message means a proxy
        // rewrote something, or the server is not the one we think it is.
        handlers.onError('server sent a non-binary websocket message')
        return
      }
      try {
        handlers.onFrame(decodeFrame(event.data))
      } catch (err) {
        handlers.onError(err instanceof Error ? err.message : String(err))
      }
    }
  }

  send(request: string): void {
    this.socket?.send(request)
  }

  close(): void {
    this.socket?.close()
    this.socket = null
  }
}
