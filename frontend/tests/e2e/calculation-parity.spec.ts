import {expect,test} from '@playwright/test'
import {launch,Listener} from './harness'
import {McpClient} from './mcp'
import type {QueryTable} from '../../src/generated/QueryTable'
import type {RecordTable} from '../../src/generated/RecordTable'
import type {SharedWireMeta} from '../../src/generated/SharedWireMeta'

const fixture='crates/kglite-visual-core/tests/fixtures/viewer-identity.kgl'

test('HTTP, MCP and WebSocket calculations share exact frozen fields and ordered revisions',async()=>{
  const server=await launch(fixture)
  const wsUrl=`${server.info.url.replace(/^http/,'ws')}ws`
  const first=new Listener(wsUrl),peer=new Listener(wsUrl)
  try{
    await Promise.all([first.open(),peer.open()])
    const post=(route:string,body:unknown)=>fetch(`${server.info.url}api/${route}`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)})
    const query=async(query:string)=>(await(await post('cypher',{query,as_graph:false})).json()) as QueryTable
    const nodes=(await query('MATCH (n) RETURN n')).row_references.flatMap(row=>row.nodes).sort((a,b)=>a.node_id-b.node_id)
    const relationships=(await query('MATCH (a)-[r]->(b) RETURN r')).row_references.flatMap(row=>row.relationships).sort((a,b)=>a.edge_id-b.edge_id).slice(0,3)
    const event=async(listener:Listener,id:string):Promise<SharedWireMeta>=>{
      const result=await listener.waitFor(done=>done.kind==='shared-update'&&done.value.meta.request_id===id)
      if(result.kind!=='shared-update')throw new Error('shared event required')
      return result.value.meta
    }
    const both=async(id:string)=>{
      const [left,right]=await Promise.all([event(first,id),event(peer,id)])
      expect(left).toEqual(right)
      return left.snapshot
    }
    expect((await post('load-entities',{nodes,relationships,request_id:'load'})).ok).toBe(true)
    const loaded=await both('load')
    const sourceRead=async()=>(await(await post('records',{handles:nodes,fields:['title','degree','@derived:degree:total']})).json()) as RecordTable
    const sourceBefore=await sourceRead()
    const response=await post('calculate',{kind:'degree',expected:loaded.stamp,request_id:'degree'})
    expect(response.status).toBe(200)
    const degree=await both('degree')
    expect(degree.calculations).toHaveLength(1)
    expect(degree.calculations[0]).toMatchObject({kind:'degree',scope:'visible',input_stamp:loaded.stamp,input_subset_revision:loaded.subset_revision,node_count:4,edge_count:3,status:'ready'})
    const originalId=degree.calculations[0]!.id
    const fields=degree.calculations[0]!.fields.map(definition=>definition.field)
    const read=async(field_refs:typeof fields)=>(await(await post('records',{handles:nodes,fields:[],field_refs})).json()) as RecordTable
    const values=(table:RecordTable)=>table.rows.map(row=>row.cells.map(cell=>{
      expect(cell.state).toBe('value')
      if(cell.state!=='value'||cell.value.type!=='int64')throw new Error('exact integer required')
      return cell.value.value
    }))
    expect(values(await read(fields))).toEqual([['0','2','2'],['3','1','4'],['0','0','0'],['0','0','0']])
    expect((await sourceRead()).rows.map(row=>row.cells)).toEqual(sourceBefore.rows.map(row=>row.cells))
    const mcp=new McpClient(server.info.mcp);await mcp.initialize()
    const components=await mcp.call('calculate',{kind:'weak-components',expected:degree.stamp,request_id:'components'})
    expect(components.isError).toBe(false)
    const withComponents=await both('components')
    expect(components.json()).toMatchObject({state:{stamp:withComponents.stamp}})
    const componentMeta=withComponents.calculations.find(value=>value.kind==='weak-components')!
    const componentRows=values(await read(componentMeta.fields.map(value=>value.field)))
    expect(componentRows.map(row=>row[1])).toEqual(['2','2','1','1'])
    expect(componentRows[0]![0]).toBe(componentRows[1]![0])
    expect(new Set(componentRows.map(row=>row[0])).size).toBe(3)
    const predicate={kind:'numeric-range',field:fields[2],min:{type:'int64',value:'3'},max:null,include_null:false,include_missing:false}
    expect((await post('subset',{predicates:[{id:'frozen-degree',enabled:true,predicate}],expected:withComponents.stamp,request_id:'filter'})).ok).toBe(true)
    const filtered=await both('filter')
    expect(filtered.subset.visible_nodes).toEqual([nodes[1]])
    expect(filtered.subset.visible_edge_ids).toEqual([2])
    first.send({type:'calculate',kind:'degree',expected:filtered.stamp,request_id:'recompute'})
    const current=await both('recompute')
    expect(values(await read(fields))[1]).toEqual(['3','1','4'])
    const fresh=current.calculations.find(value=>value.kind==='degree'&&value.id!==originalId)!
    const freshTable=await read(fresh.fields.map(value=>value.field))
    expect(freshTable.rows[1]!.cells.map(cell=>cell.state==='value'?cell.value:null)).toEqual([{type:'int64',value:'1'},{type:'int64',value:'1'},{type:'int64',value:'2'}])
    expect(current.subset.visible_nodes).toEqual(filtered.subset.visible_nodes)
    first.send({type:'calculate',kind:'degree',calculation_id:originalId,expected:current.stamp,request_id:'replace-original'})
    const replaced=await both('replace-original')
    expect(replaced.calculations).toHaveLength(3)
    expect(replaced.subset.visible_nodes).toEqual([])
    expect(replaced.calculations.find(value=>value.id===originalId)).toMatchObject({input_subset_revision:current.subset_revision,node_count:1,edge_count:1})
    expect((await post('calculate',{kind:'degree',calculation_id:'unknown-result',expected:replaced.stamp})).status).toBe(400)
    const stale={kind:'degree' as const,expected:loaded.stamp,request_id:'stale'}
    expect((await post('calculate',stale)).status).toBe(409)
    expect((await mcp.call('calculate',stale)).isError).toBe(true)
    first.send({type:'calculate',...stale})
    const failure=await first.waitFor(done=>done.kind==='error'&&done.request_id==='stale')
    expect(failure.kind).toBe('error')
    expect(first.received.filter(done=>done.kind==='shared-update'&&done.value.meta.request_id==='stale')).toEqual([])
    const late=new Listener(wsUrl)
    try{
      await late.open()
      const baseline=late.received.find(done=>done.kind==='shared-update')
      expect(baseline?.kind).toBe('shared-update')
      if(baseline?.kind==='shared-update')expect(baseline.value.meta.snapshot.calculations).toEqual(replaced.calculations)
    }finally{late.close()}
    for(const listener of [first,peer])for(const id of ['load','degree','components','filter','recompute','replace-original']){
      expect(listener.received.filter(done=>done.kind==='shared-update'&&done.value.meta.request_id===id)).toHaveLength(1)
    }
  }finally{first.close();peer.close();server.process.kill()}
})
