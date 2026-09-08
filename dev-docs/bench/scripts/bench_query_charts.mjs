/** Real-app query-chart correctness and first-event capture.
 *
 * Run only after a production frontend build has been embedded in the resolved
 * binary. First-chart and query-table timings are means of first events; the
 * fixed integer loop immediately around each pair is the machine-drift control.
 */
import {execFileSync, spawn} from 'node:child_process'
import {createInterface} from 'node:readline'
import {createRequire} from 'node:module'
import {mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync} from 'node:fs'
import {createHash} from 'node:crypto'
import os from 'node:os'
import path from 'node:path'
import {fileURLToPath} from 'node:url'

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const require = createRequire(path.join(REPO, 'frontend/package.json'))
const {chromium} = require('@playwright/test')
const graph = path.resolve(REPO, process.argv[2] ?? 'dev-docs/bench/out/sodir-showcase/fresh-workdir/sodir-notebook.kgl')
const output = path.resolve(REPO, process.argv[3] ?? 'dev-docs/bench/results/query-result-charts/real-browser.json')
const captures = path.resolve(REPO, 'dev-docs/temp/chart-browser-captures'); mkdirSync(captures, {recursive:true})
const config = mkdtempSync(path.join(os.tmpdir(), 'kglv-chart-bench-'))
const binary = execFileSync('python3', ['scripts/check_bundle.py','--resolve-binary','kglite-visual'], {cwd:REPO,encoding:'utf8'}).trim()
const child = spawn(binary,[graph,'--no-open','--port','0'],{cwd:REPO,env:{...process.env,KGLITE_VISUAL_CONFIG_DIR:config}})
const errors=[]; createInterface({input:child.stderr}).on('line',line=>errors.push(line))
const info = await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(new Error(errors.join('\n'))),30000);createInterface({input:child.stdout}).once('line',line=>{clearTimeout(timer);resolve(JSON.parse(line))})})
const browser = await chromium.launch({headless:true,args:['--use-gl=angle','--use-angle=swiftshader','--enable-unsafe-swiftshader']})

async function fillQuery(page,text){
  await page.getByTestId('destination-query').click()
  await page.locator('[data-testid="query-editor"] .cm-content, [data-testid="editor-note"].kglv-warn').first().waitFor({state:'attached'})
  await page.locator('[data-testid="query-editor"] .cm-content, [data-testid="query-editor"] textarea').first().fill(text)
}
async function control(page){return page.evaluate(()=>{const start=performance.now();let x=0;for(let i=0;i<12_000_000;i++)x=(x+i)|0;return {ms:performance.now()-start,value:x}})}
async function queryAndBuild(page,query,expectedRows,configure=async()=>{}){
  await fillQuery(page,query)
  await page.evaluate(()=>{window.__chartQueryChangedAt=null;const node=document.querySelector('[data-testid="query-status"]');new MutationObserver((_,o)=>{window.__chartQueryChangedAt=performance.now();o.disconnect()}).observe(node,{childList:true,subtree:true,characterData:true})})
  const queryStart=await page.evaluate(()=>performance.now()); await page.getByTestId('query-run').click()
  await page.waitForFunction(rows=>window.__kglv.queryRows===rows && window.__chartQueryChangedAt!==null,expectedRows)
  const tableMs=await page.evaluate(start=>window.__chartQueryChangedAt-start,queryStart)
  await page.getByTestId('chart-open').click(); await configure(page)
  await page.evaluate(()=>{window.__chartPaintAt=null;const node=document.querySelector('[data-testid="chart-picture"]');new MutationObserver((_,o)=>{if(node.querySelector('svg')){window.__chartPaintAt=performance.now();o.disconnect()}}).observe(node,{childList:true,subtree:true})})
  const chartStart=await page.evaluate(()=>performance.now()); await page.getByTestId('chart-build').click()
  await page.waitForFunction(()=>window.__chartPaintAt!==null)
  return {table_ms:tableMs,chart_ms:await page.evaluate(start=>window.__chartPaintAt-start,chartStart),status:await page.getByTestId('chart-status').textContent()}
}

const page=await browser.newPage({viewport:{width:1440,height:900}}); const consoleErrors=[];page.on('console',m=>{if(m.type()==='error')consoleErrors.push(m.text())})
const bundleFiles=readdirSync(path.join(REPO,'frontend/dist/assets')).sort();const bundleHash=createHash('sha256');for(const name of bundleFiles){bundleHash.update(name);bundleHash.update(readFileSync(path.join(REPO,'frontend/dist/assets',name)))}
const result={head:execFileSync('git',['rev-parse','HEAD'],{cwd:REPO,encoding:'utf8'}).trim(),dirty_files:execFileSync('git',['status','--short'],{cwd:REPO,encoding:'utf8'}).trim().split('\n').filter(Boolean),bundle_sha256:bundleHash.digest('hex'),bundle_files:bundleFiles,binary,graph,info,started_at:new Date().toISOString(),platform:{platform:process.platform,arch:process.arch,cpus:os.cpus().length,loadavg:os.loadavg()},runs:[],controls:[],artifacts:{},console_errors:consoleErrors}
try{
 await page.goto(`${info.url}?deterministic=1`);await page.waitForFunction(()=>window.__kglv?.ready===true)
 result.graph_before=await page.evaluate(()=>({...window.__kglv}))
 const production=`MATCH (p:ProductionProfile)-[:OF_FIELD]->(f:Field) WHERE f.title IN ['JOHAN SVERDRUP','TROLL','EKOFISK'] WITH f,p UNWIND ts_series(p.prd_oil_net,'2020','2024') AS point RETURN f.title AS field,point.time AS date,point.value AS oil ORDER BY field,date LIMIT 500`
 for(let run=1;run<=2;run++){
  result.controls.push({case:'production-180',run,before:await control(page)})
  result.runs.push({case:'production-180',run,...await queryAndBuild(page,production,180,async p=>{
    await p.getByTestId('chart-title').fill('Historical oil production by field');await p.getByTestId('chart-x-label').fill('Month');await p.getByTestId('chart-y-label').fill('Oil production');await p.getByTestId('chart-unit').fill('Sm³/day');await p.getByTestId('chart-monthly').check();await p.getByTestId('chart-monthly-confirm').check();await p.getByTestId('chart-scale').fill('1000000')
  })})
  result.controls.at(-1).after=await control(page)
 }
 await page.screenshot({path:path.join(captures,'sodir-query-chart-desktop.png'),fullPage:true});result.artifacts.desktop=path.join(captures,'sodir-query-chart-desktop.png')
 await page.getByTestId('chart-values').locator('summary').click(); result.value_summary=await page.getByTestId('chart-values').textContent()
 await page.getByTestId('chart-table-view').click(); result.table_rows=await page.getByTestId('query-table').locator('tr').count(); result.graph_after_table=await page.evaluate(()=>({...window.__kglv}))
 await page.getByTestId('chart-chart-view').click()
 for(const [id,ext] of [['chart-svg','svg'],['chart-png','png']]){const started=Date.now();const [download]=await Promise.all([page.waitForEvent('download'),page.getByTestId(id).click()]);if(ext==='png')result.png_export_first_ms=Date.now()-started;const target=path.join(captures,`sodir-production-chart.${ext}`);await download.saveAs(target);result.artifacts[ext]=target}
 await page.setViewportSize({width:720,height:900});await page.screenshot({path:path.join(captures,'sodir-query-chart-narrow.png'),fullPage:true});result.artifacts.narrow=path.join(captures,'sodir-query-chart-narrow.png');result.narrow_overflow=await page.evaluate(()=>({document:document.documentElement.scrollWidth-document.documentElement.clientWidth,chart:document.querySelector('[data-testid="chart-picture"]')?.scrollWidth-document.querySelector('[data-testid="chart-picture"]')?.clientWidth}))
 await page.setViewportSize({width:1440,height:900})
 const bound=`UNWIND range(1,5000) AS sequence RETURN sequence, (sequence % 101) AS value LIMIT 5000`
 for(let run=1;run<=2;run++){result.controls.push({case:'bound-5000',run,before:await control(page)});result.runs.push({case:'bound-5000',run,...await queryAndBuild(page,bound,5000)});result.controls.at(-1).after=await control(page)}
 result.graph_after=await page.evaluate(()=>({...window.__kglv}));result.finished_at=new Date().toISOString()
 const mean=(key,name)=>result.runs.filter(r=>r.case===name).reduce((a,r)=>a+r[key],0)/2
 result.means={production_180:{table_ms:mean('table_ms','production-180'),chart_ms:mean('chart_ms','production-180')},bound_5000:{table_ms:mean('table_ms','bound-5000'),chart_ms:mean('chart_ms','bound-5000')}}
 writeFileSync(output,JSON.stringify(result,null,2));console.log(JSON.stringify({output,means:result.means,artifacts:result.artifacts,narrow_overflow:result.narrow_overflow}))
}finally{await browser.close();child.kill('SIGTERM');await new Promise(resolve=>child.once('exit',resolve));rmSync(config,{recursive:true,force:true})}
