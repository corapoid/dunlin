// Production React, native cookie HTTP/catalog WS and real local linked Git.
// Stdio changes only trusted operator metadata/grants or a concurrent binding.
import assert from 'node:assert/strict';
import { createInterface } from 'node:readline';
import { chromium } from 'playwright-core';
const lines=createInterface({input:process.stdin,crlfDelay:Infinity});
const input=lines[Symbol.asyncIterator]();
async function receive(){const line=await input.next();assert.equal(line.done,false);return JSON.parse(line.value);}
const fixture=await receive();
async function source(command){process.stdout.write(`${JSON.stringify({command})}\n`);assert.deepEqual(await receive(),{ok:command});}
const path=`/v1/web/workspaces/${fixture.workspace}/projects/${fixture.project}/binding`;
const browser=await chromium.launch({executablePath:process.env.DUNLIN_CHROMIUM || chromium.executablePath(),headless:true,args:['--no-sandbox','--disable-dev-shm-usage']});
try {
  const errors=[],writes=[];let catalogFrames=0;
  const observations=new WeakMap();
  function notify(observation) {
    for(const waiter of [...observation.waiters]) {
      if(waiter.ready()) {observation.waiters.delete(waiter);clearTimeout(waiter.timer);waiter.resolve();}
    }
  }
  function causalCatalogRead(page, previous, revision) {
    const observation=observations.get(page);
    const ready=()=>observation.catalog!==null && observation.catalog!==previous
      && observation.reads.some(read=>read.catalog===observation.catalog && read.bindingRevision===revision);
    if(ready())return Promise.resolve();
    return new Promise((resolve,reject)=>{
      const waiter={ready,resolve,timer:null};
      waiter.timer=setTimeout(()=>{
        observation.waiters.delete(waiter);
        reject(new Error(`Missing native catalog revision and subsequent binding GET for revision ${revision}`));
      },15000);
      observation.waiters.add(waiter);
    });
  }
  async function pair(code) {
    const context=await browser.newContext({viewport:{width:1280,height:1000},reducedMotion:'reduce'});
    const page=await context.newPage();page.setDefaultTimeout(15000);
    const observation={catalog:null,requests:new WeakMap(),reads:[],waiters:new Set()};
    observations.set(page,observation);
    page.on('pageerror',error=>errors.push(error.message));
    page.on('request',request=>{
      assert.equal(new URL(request.url()).origin,fixture.origin);
      if(new URL(request.url()).pathname===path) {
        if(request.method()==='GET')observation.requests.set(request,observation.catalog);
        if(request.method()==='PUT')writes.push(JSON.parse(request.postData()));
      }
    });
    page.on('websocket',socket=>{
      assert.equal(new URL(socket.url()).host,new URL(fixture.origin).host);
      socket.on('framereceived',({payload})=>{
        catalogFrames++;
        if(new URL(socket.url()).pathname!=='/v1/web/live')return;
        const event=JSON.parse(String(payload));
        if(event.type==='snapshot') {
          observation.catalog=`${event.snapshot.stream_id}:${event.snapshot.workspace_revision ?? 0}`;
          notify(observation);
        }
      });
    });
    page.on('response',async response=>{
      if(new URL(response.url()).pathname!==path || response.request().method()!=='GET' || response.status()!==200)return;
      try {
        const value=await response.json();
        observation.reads.push({catalog:observation.requests.get(response.request()),bindingRevision:value.binding?.revision});
        if(observation.reads.length>64)observation.reads.shift();
        notify(observation);
      } catch { /* A catalog remount may abort a superseded read body. */ }
    });
    await page.goto(fixture.origin);
    await page.getByLabel('Jednorazowy kod parowania').fill(code);
    await page.getByRole('button',{name:'Połącz bezpiecznie',exact:true}).click();
    await page.getByRole('button',{name:'Zespół',exact:true}).click();
    await page.locator(`[data-testid="team-workspace"][data-workspace-id="${fixture.workspace}"]`).click();
    await page.locator(`[data-testid="project-local-binding"][data-project-id="${fixture.project}"]`).click();
    await page.getByTestId('local-binding-refresh').waitFor();
    return page;
  }
  const page=await pair(fixture.ownerCode);
  const panel=page.getByTestId('local-binding');
  async function read(p=page){return p.evaluate(async path=>{const r=await fetch(path,{credentials:'same-origin',cache:'no-store'});return{status:r.status,value:await r.json()};},path);}
  async function refresh(p=page){await p.getByTestId('local-binding-refresh').click();await p.waitForFunction(()=>document.querySelector('[data-testid="local-binding-refresh"]')?.disabled===false);}
  async function save(native,approval,status=200){
    const previousCatalog=observations.get(page).catalog;
    await panel.getByTestId('local-binding-native').fill(native);
    await panel.getByTestId('local-binding-approval').fill(approval);
    const pending=page.waitForResponse(r=>new URL(r.url()).pathname===path && r.request().method()==='PUT');
    await panel.getByTestId('local-binding-save').click();const response=await pending;
    assert.equal(response.status(),status);
    const accepted=await response.json();
    if(status===200) {
      // The mutation receipt alone precedes the catalog invalidation. Wait for
      // its native frame and a binding GET begun after that frame, so a late
      // component remount cannot clear the next intent while it is being typed.
      await causalCatalogRead(page,previousCatalog,accepted.revision);
      await panel.getByTestId('local-binding-native').waitFor();
    }
    return accepted;
  }
  const ordinary=await save(fixture.ordinary,'');
  assert.equal(ordinary.repo_kind,'git');assert.equal(ordinary.revision,1);
  assert.deepEqual(writes[0],{native_project_id:fixture.ordinary,expected_revision:null});
  await refresh();
  const linked=await save(fixture.linked,'1');
  for (const binding of [ordinary,linked]) {
    for (const field of ['root','path','identity','git_dir','common_dir','execution_ready']) assert.equal(field in binding,false,`Binding exposes unexpected ${field}`);
  }
  assert.equal(linked.repo_kind,'linked_worktree');assert.equal(linked.metadata_approval_revision,1);assert.equal(linked.revision,2);
  assert.deepEqual(writes[1],{native_project_id:fixture.linked,metadata_approval_revision:1,expected_revision:1});
  await refresh();
  await panel.getByTestId('local-binding-current').getByText('Konfiguracja bieżąca dla Twojego dostępu do odczytu.',{exact:true}).waitFor();
  const viewer=await pair(fixture.viewerCode);
  await viewer.getByTestId('local-binding-saved').getByText(fixture.linked,{exact:false}).waitFor();
  for(const id of ['local-binding-native','local-binding-approval','local-binding-save','local-binding-revoke'])assert.equal(await viewer.getByTestId(id).count(),0,`${id} is management-only`);
  const viewerRead=await read(viewer);assert.equal(viewerRead.status,200);assert.equal(viewerRead.value.can_manage,false);assert.equal(viewerRead.value.configuration_current,false);
  // A concurrent operator binding changes the catalog. Draft and captured CAS
  // must clear; an actual stale request still receives 409 without any retry.
  await panel.getByTestId('local-binding-native').fill('DRAFT_MUST_CLEAR');
  await panel.getByTestId('local-binding-approval').fill('9');
  const beforeConcurrent=observations.get(page).catalog;
  await source('bump_binding');
  await causalCatalogRead(page,beforeConcurrent,3);
  await panel.getByTestId('local-binding-saved').getByText('Wersja 3',{exact:false}).waitFor();
  assert.equal(await panel.getByTestId('local-binding-native').inputValue(),fixture.linked);
  assert.equal(await panel.getByTestId('local-binding-approval').inputValue(),'1');
  const stale=await page.evaluate(async({path,native})=>{const r=await fetch(path,{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify({native_project_id:native,expected_revision:2,metadata_approval_revision:1})});return{status:r.status,value:await r.json()};},{path,native:fixture.linked});
  assert.equal(stale.status,409);assert.equal((await read()).value.binding.revision,3);
  await refresh();
  assert.equal(writes.length,3,'Two UI saves plus one explicit stale CAS request; no hidden write retries');
  await source('reapprove');
  await refresh();
  const old=await read();assert.equal(old.value.binding.metadata_approval_revision,1);assert.equal(old.value.configuration_current,false);
  await save(fixture.linked,'1',403);
  await panel.getByTestId('local-binding-save').waitFor({state:'detached'});
  await refresh();
  const rebound=await save(fixture.linked,'3');assert.equal(rebound.revision,4);assert.equal(rebound.metadata_approval_revision,3);
  await refresh();
  await source('revoke_read');await refresh();
  const denied=await read();assert.equal(denied.status,200);assert.equal(denied.value.binding.revision,4);assert.equal(denied.value.configuration_current,false);
  await panel.getByTestId('local-binding-saved').getByText(fixture.linked,{exact:false}).waitFor();
  await panel.getByTestId('local-binding-current').getByText('Konfiguracja nie została potwierdzona jako bieżąca dla Twojego dostępu.',{exact:true}).waitFor();
  await source('restore_read');await refresh();
  assert.equal((await read()).value.configuration_current,true);
  assert.equal(writes.length,5,'Metadata revocation/refresh never silently rebinds or retries a write');
  await panel.getByTestId('local-binding-native').fill('AUTH_DRAFT_MUST_CLEAR');
  await source('revoke_device');
  await page.getByLabel('Jednorazowy kod parowania').waitFor();
  assert.equal(await page.getByTestId('local-binding').count(),0);
  assert.equal((await page.locator('body').innerText()).includes('AUTH_DRAFT_MUST_CLEAR'),false);
  assert.ok(catalogFrames>0,'Catalog updates must use actual native WebSocket traffic');
  assert.deepEqual(errors,[]);
  console.log('PASS Chromium local bindings: production UI ordinary/linked CAS saves, explicit operator pin, native catalog draft clearing, stale CAS denied, reapproval requires rebind, own Read revoke preserves saved configuration, viewer read-only and revoked identity clears drafts. No execution readiness claim.');
} finally {await browser.close();lines.close();process.stdin.destroy();}
