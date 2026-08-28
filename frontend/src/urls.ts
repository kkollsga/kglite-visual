/**
 * Every URL this app builds is relative to the page it was served from.
 *
 * The same bundle is served by the CLI at `/`, by the Python wheel, and by
 * jupyter-server-proxy under a prefix such as `/proxy/8731/`. An absolute path
 * works in the first two and 404s in the third — while the page itself still
 * loads, so the failure reads as a server bug. Building the rule in from the
 * first commit costs nothing; retrofitting it means auditing every fetch and
 * every socket in the app (plan D7).
 */

/** Directory of the current document, always with a trailing slash. */
function baseDir(): string {
  const path = window.location.pathname
  return path.endsWith('/') ? path : path.slice(0, path.lastIndexOf('/') + 1)
}

/** An HTTP endpoint under whatever prefix served this page. */
export function apiUrl(relativePath: string): string {
  return new URL(relativePath, window.location.origin + baseDir()).toString()
}

/** The WebSocket endpoint, protocol-matched so an https deployment works. */
export function wsUrl(relativePath: string): string {
  const scheme = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const url = new URL(relativePath, window.location.origin + baseDir())
  url.protocol = scheme
  return url.toString()
}
