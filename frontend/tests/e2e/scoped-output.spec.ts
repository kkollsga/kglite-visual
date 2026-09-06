import { expect, test } from '@playwright/test'
import { launch, Listener } from './harness'
import { McpClient } from './mcp'
import type { QueryTable } from '../../src/generated/QueryTable'

const fixture = 'crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl'

test('captured HTTP/MCP export preserves exact relation multiplicity, identity and preview conflicts', async () => {
  const server = await launch(fixture)
  const observer = new Listener(`${server.info.url.replace(/^http/, 'ws')}ws`)
  try {
    await observer.open()
    const post = (route: string, data: unknown) => fetch(`${server.info.url}api/${route}`, {
      method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify(data),
    })
    const query = async (text: string) => (await (await post('cypher', {query: text, as_graph: false})).json()) as QueryTable
    const nodes = (await query('MATCH (n:Person) RETURN n')).row_references.flatMap(row => row.nodes)
    const relationships = (await query('MATCH (a)-[r]->(b) RETURN r')).row_references.flatMap(row => row.relationships).sort((a,b) => a.edge_id-b.edge_id).slice(0,3)
    expect((await post('load-entities', {nodes,relationships,request_id:'output-load'})).ok).toBe(true)
    await observer.waitFor(done => done.kind === 'shared-update' && done.value.meta.request_id === 'output-load')
    observer.received.length = 0
    const before = await (await fetch(`${server.info.url}api/view-state`)).json()
    const request = {scope:'visible',expected:before.stamp,subset_revision:before.subset_revision,format:'gexf',include_identity:true}
    const previewResponse = await post('export/preview',request)
    expect(previewResponse.status).toBe(200)
    const preview = await previewResponse.json()
    expect(preview).toMatchObject({nodes:3,edges:3,scope:'visible',stamp:before.stamp})
    expect([...preview.identity.edge_ids].sort()).toEqual([0,1,2])
    expect(preview.identity.semantics).toContain('export-local')
    const finalRequest = {...request,preview_digest:preview.preview_digest}
    const artifact = await post('export/download',finalRequest)
    expect(artifact.status).toBe(200)
    expect(artifact.headers.get('x-kglv-scope')).toBe('visible')
    expect(artifact.headers.get('content-disposition')).toContain('.gexf')
    const xml = await artifact.text()
    const pairs = [...xml.matchAll(/<edge id="[^"]+" source="([^"]+)" target="([^"]+)"/g)].map(match => [match[1],match[2]])
    expect(pairs).toHaveLength(3)
    expect(pairs.filter(([source,target]) => source === target)).toHaveLength(1)
    const legacy = await (await fetch(`${server.info.url}api/export?format=gexf&source=live-view`)).text()
    expect([...legacy.matchAll(/<edge id=/g)]).toHaveLength(4)
    expect((await post('export/download',{...finalRequest,format:'csv'})).status).toBe(409)
    expect((await post('export/download',request)).status).toBe(400)

    const mcp = new McpClient(server.info.mcp); await mcp.initialize()
    expect((await mcp.call('export_view',{scope:'visible'})).isError).toBe(true)
    const mcpPreview = await mcp.call('export_view',request)
    expect(mcpPreview.isError).toBe(false)
    expect(mcpPreview.json()).toMatchObject({...preview,output_stage:'preview'})
    const mcpFinal = await mcp.call('export_view',finalRequest)
    expect(mcpFinal.isError).toBe(false)
    expect(JSON.parse(mcpFinal.text.slice(0, -xml.length))).toMatchObject({preview_digest:preview.preview_digest,output_stage:'final'})
    expect(mcpFinal.text).toContain(xml)
    expect(await (await fetch(`${server.info.url}api/view-state`)).json()).toEqual(before)
    expect(observer.received).toEqual([])
    expect((await post('caption',{caption_by:'id',request_id:'stale-output'})).ok).toBe(true)
    await observer.waitFor(done => done.kind === 'shared-update' && done.value.meta.request_id === 'stale-output')
    expect((await post('export/download',finalRequest)).status).toBe(409)
    expect((await mcp.call('export_view',finalRequest)).isError).toBe(true)
  } finally { observer.close();server.process.kill() }
})

test('server image JSON accounts for base64 and uses exactly the preview settings for final bytes', async () => {
  const server = await launch(fixture)
  try {
    const post = (route: string, data: unknown) => fetch(`${server.info.url}api/${route}`, {
      method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify(data),
    })
    expect((await post('browse-type',{node_type:'Person'})).ok).toBe(true)
    const before = await (await fetch(`${server.info.url}api/view-state`)).json()
    const body = {scope:'visible',expected:before.stamp,subset_revision:before.subset_revision,format:'png',width:400,height:300,seed:42,theme:'light',kernel:'force'}
    const response = await post('render/preview',body)
    expect(response.status).toBe(200)
    const preview = await response.json()
    expect(preview.rendered).toMatchObject({width:400,height:300,format:'png'})
    expect(preview.preview.notes.join(' ')).toContain('deterministic server image')
    const expected = Buffer.from(preview.image_base64,'base64')
    expect(expected.byteLength).toBeLessThanOrEqual(16*1024*1024)
    const finalBody = {...body,preview_digest:preview.preview.preview_digest}
    const final = await post('render/download',finalBody)
    expect(final.status).toBe(200)
    expect(Buffer.from(await final.arrayBuffer())).toEqual(expected)
    expect((await post('render/download',{...finalBody,width:401})).status).toBe(409)
    const mcp = new McpClient(server.info.mcp); await mcp.initialize()
    const {kernel,...mcpBody} = body
    const image = await mcp.call('render',{...mcpBody,layout:kernel})
    expect(image.isError).toBe(false)
    expect(image.json()).toMatchObject({preview:{preview_digest:preview.preview.preview_digest},output_stage:'preview'})
    expect(Buffer.from(image.images[0]!.base64,'base64')).toEqual(expected)
    expect(await (await fetch(`${server.info.url}api/view-state`)).json()).toEqual(before)
  } finally { server.process.kill() }
})
