// Rendered production UI, real REST/AHP/controller/runner, controlled OAuth/MCP provider.
import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';
let input='';for await(const part of process.stdin)input+=part;
const fixture=JSON.parse(input);
const browser=await chromium.launch({executablePath:process.env.DUNLIN_CHROMIUM,headless:true,args:['--no-sandbox','--disable-dev-shm-usage']});
let page;
try {
  const context=await browser.newContext({viewport:{width:1440,height:1000}});
  page=await context.newPage();page.setDefaultTimeout(15000);
  const errors=[],attempts=[],targetRequests=[],urlDecisions=[];
  const urlInput=fixture.form_action?.startsWith('url-');
  const scoped=fixture.selection.provider_id.startsWith('dunlin-secret-');
  page.on('pageerror',error=>errors.push(error.message));
  page.on('websocket',socket=>socket.on('framesent',({payload})=>{
    const value=JSON.parse(String(payload));
    if(urlInput&&value.method==='dispatchAction'&&value.params.action.type==='chat/inputCompleted'){
      assert.equal(Object.hasOwn(value.params.action,'answers'),false);urlDecisions.push(value.params.action);
    }
  }));
  context.on('request',request=>{
    if(new URL(request.url()).origin===fixture.origin)return;
    assert.ok(urlInput);assert.match(request.url(),/^https:\/\/review\.example\.test\/connect\?step=[12]$/);
    targetRequests.push(request.url());
    for(const header of ['authorization','cookie','referer'])assert.equal(request.headers()[header],undefined);
  });
  await context.route('https://review.example.test/**',route=>route.fulfill({status:200,contentType:'text/html',body:'<!doctype html><title>External review fixture</title><p>Controlled external page</p>'}));
  await page.goto(fixture.origin);
  await page.getByLabel('Jednorazowy kod parowania').fill(fixture.ownerCode);
  await page.getByRole('button',{name:'Połącz bezpiecznie',exact:true}).click();
  let scopedDesktop,scopedMobile;
  if(scoped) {
    await page.getByRole('button',{name:'Poświadczenia',exact:true}).click();
    await page.getByTestId('agent-workspace').selectOption(fixture.workspace);
    await page.getByTestId('agent-project').selectOption(fixture.project);
    await page.getByTestId('scoped-host').selectOption(fixture.host);
    await page.getByTestId('scoped-name').fill('browser-ui-api');
    await page.getByTestId('scoped-value').fill('browser-config-private-value');
    await page.getByTestId('scoped-domain').fill('worker.tail123.ts.net');
    await page.getByTestId('scoped-path').fill('/items');
    let credentialWrites=0;
    await page.route('**/scoped-secrets/browser-ui-api',async route=>{
      if(route.request().method()!=='POST')return route.continue();
      credentialWrites++;
      const response=await route.fetch();assert.equal(response.status(),200);
      await route.abort('failed'); // Effect committed, browser lost its receipt.
    });
    await page.getByTestId('scoped-save').click();
    await page.getByRole('alert').filter({hasText:'Nie udało się potwierdzić operacji poświadczenia.'}).waitFor();
    assert.equal(await page.getByTestId('scoped-value').inputValue(),'');
    assert.equal(await page.getByTestId('scoped-save').isDisabled(),true);
    assert.equal(credentialWrites,1);
    await page.getByTestId('scoped-reload').click();
    await page.getByTestId('scoped-credential').locator('option[value="browser-ui-api"]').waitFor({state:'attached'});
    assert.equal(credentialWrites,1);
    await page.unroute('**/scoped-secrets/browser-ui-api');
    await page.getByTestId('scoped-credential').selectOption('browser-ui-api');
    await page.getByTestId('scoped-profile').selectOption(fixture.profile.agent_id);
    await page.getByTestId('scoped-slot').selectOption(fixture.selection.provider_id);
    assert.equal(await page.getByTestId('scoped-principal').inputValue(),fixture.user);
    assert.equal(await page.getByTestId('scoped-readonly').isChecked(),true);
    await page.getByTestId('scoped-grant-save').click();
    await page.getByRole('status').filter({hasText:'Grant zapisano'}).waitFor();
    await page.locator('[data-testid^="scoped-revoke-"]').click();
    await page.getByRole('status').filter({hasText:'Grant odwołano.'}).waitFor();
    await page.getByTestId('scoped-disable').click();
    await page.getByRole('status').filter({hasText:'Poświadczenie wyłączono.'}).waitFor();
    assert.ok(!(await page.locator('body').innerText()).includes('browser-config-private-value'));
    for(const storage of [await page.evaluate(()=>({...localStorage})),await page.evaluate(()=>({...sessionStorage}))])assert.ok(!JSON.stringify(storage).includes('browser-config-private-value'));
    await page.waitForFunction(()=>document.querySelector('[data-testid="scoped-credential"]')?.selectedOptions[0]?.textContent.includes('wyłączone'));
    scopedDesktop=(await page.screenshot({fullPage:true})).toString('base64');
    await page.setViewportSize({width:390,height:844});
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth));
    scopedMobile=(await page.screenshot({fullPage:true})).toString('base64');
    await page.setViewportSize({width:1440,height:1000});
    await page.getByRole('button',{name:'Agenci',exact:true}).click();
    await page.getByTestId('agent-workspace').selectOption(fixture.workspace);
    await page.getByTestId('agent-project').selectOption(fixture.project);
    await page.getByTestId('agent-name').fill('Scoped HTTP browser builder');
    await page.getByTestId('agent-harness').selectOption('opencode');
    await page.getByTestId('agent-model').fill('fixture-model');
    await page.getByTestId('agent-mode').fill('fixture-mode');
    await page.getByTestId('agent-instructions').fill('literal-instructions');
    await page.getByTestId('agent-skill-add').click();
    await page.getByLabel('Identyfikator umiejętności',{exact:true}).fill('fixture');
    await page.getByLabel('Wersja umiejętności',{exact:true}).fill('1');
    await page.getByLabel('Instrukcje umiejętności',{exact:true}).fill('literal-skill');
    await page.getByLabel('Limit czasu (sekundy)',{exact:true}).fill('60');
    await page.getByLabel('Limit wyjścia (bajty)',{exact:true}).fill('65536');
    await page.getByText('Dodaj dostęp HTTP z poświadczeniem',{exact:true}).click();
    await page.getByTestId('scoped-tool-host').selectOption(fixture.host);
    await page.getByTestId('scoped-tool-slot').fill('browser');
    await page.getByTestId('scoped-tool-add').click();
    await page.getByTestId('agent-mcp-tool').waitFor();
    await page.getByTestId('agent-save').click();
    await page.getByRole('status').filter({hasText:'Szkic zapisany.'}).waitFor();
    await page.getByTestId('agent-publish').click();
    await page.getByRole('status').filter({hasText:'Opublikowano wersję 1.'}).waitFor();
    const published=await (await page.request.get(`${fixture.origin}/v1/web/workspaces/${fixture.workspace}/projects/${fixture.project}/agent-profiles`)).json();
    const created=published.profiles.find(p=>p.definition.name==='Scoped HTTP browser builder');
    assert.ok(created);fixture.profile={agent_id:created.agent_id,version:created.version,digest:created.digest};
    await page.getByRole('button',{name:'Poświadczenia',exact:true}).click();
    await page.getByTestId('agent-workspace').selectOption(fixture.workspace);
    await page.getByTestId('agent-project').selectOption(fixture.project);
    await page.getByTestId('scoped-host').selectOption(fixture.host);
    await page.getByTestId('scoped-credential').locator('option[value="bridge"]').waitFor({state:'attached'});
    await page.getByTestId('scoped-credential').selectOption('bridge');
    await page.getByTestId('scoped-profile').selectOption(created.agent_id);
    await page.getByTestId('scoped-slot').selectOption('dunlin-secret-browser');
    const receipt=page.waitForResponse(r=>r.request().method()==='POST'&&r.url().endsWith('/scoped-secrets/bridge/grants'));
    await page.getByTestId('scoped-grant-save').click();
    const grantResponse=await receipt;assert.equal(grantResponse.status(),200);
    const assigned=await grantResponse.json();
    fixture.selection={provider_id:assigned.provider_id,connection_id:assigned.id,revision:assigned.revision};
    await page.getByRole('status').filter({hasText:'Grant zapisano'}).waitFor();
  }
  await page.getByRole('button',{name:'Sesje',exact:true}).click();
  await page.getByTestId('session-create').locator('summary').click();
  await page.getByTestId('agent-workspace').selectOption(fixture.workspace);
  await page.getByTestId('agent-project').selectOption(fixture.project);
  const catalog=await (await page.request.get(`${fixture.origin}/v1/web/workspaces/${fixture.workspace}/projects/${fixture.project}/agent-profiles`)).json();
  assert.equal(catalog.remote_mcp_sessions_supported,true);
  assert.equal(catalog.mcp_sessions_supported,false);
  await page.getByTestId('session-create-target').selectOption(fixture.binding.id);
  await page.getByTestId('session-create-profile').selectOption(fixture.profile.agent_id);
  const account=()=>page.getByTestId(`session-mcp-${fixture.selection.provider_id}`);
  await account().locator(`option[value="${fixture.selection.connection_id}"]`).waitFor({state:'attached'});
  assert.equal(await account().inputValue(),'');
  assert.equal(await page.getByTestId('session-create-submit').isDisabled(),true);
  await account().selectOption(fixture.selection.connection_id);
  // Moving between targets cannot carry an account choice into a new context.
  await page.getByTestId('session-create-target').selectOption('local');
  assert.equal(await page.getByTestId('session-create-submit').isDisabled(),true);
  await page.getByTestId('session-create-target').selectOption(fixture.binding.id);
  await account().locator(`option[value="${fixture.selection.connection_id}"]`).waitFor({state:'attached'});
  assert.equal(await account().inputValue(),'');
  await account().selectOption(fixture.selection.connection_id);
  let accepted;
  await page.route('**/interactive-sessions',async route=>{
    if(route.request().method()!=='POST')return route.continue();
    attempts.push({body:route.request().postData(),key:route.request().headers()['idempotency-key']});
    const response=await route.fetch();assert.equal(response.status(),202);
    if(attempts.length===1){accepted=await response.json();await route.abort('failed');}
    else {assert.equal(response.headers()['idempotency-replayed'],'true');await route.fulfill({response});}
  });
  await page.getByTestId('session-create-submit').click();
  await page.getByTestId('session-create-retry').waitFor();
  assert.equal(attempts.length,1);
  assert.deepEqual(JSON.parse(attempts[0].body),{profile:fixture.profile,mcp_connections:[fixture.selection],target:{kind:'remote',binding:fixture.binding}});
  await page.getByTestId('session-create-retry').click();
  const id=accepted.session.id;
  await page.getByTestId('session-created-id').filter({hasText:id}).waitFor();
  await page.getByRole('alert').filter({hasText:'Wynik utworzenia sesji jest nieznany'}).waitFor({state:'detached'});
  assert.equal(attempts.length,2);assert.deepEqual(attempts[0],attempts[1]);
  await page.locator(`[data-testid="ahp-session"][data-session-uri="ahp-session:/${id}"]`).click();
  await page.getByTestId('ahp-prompt').fill('Remember nonce:remote-ui-context');
  await page.getByTestId('ahp-send').click();
  await page.getByTestId('ahp-permission').waitFor();
  await page.getByTestId('ahp-tool').locator('summary').filter({hasText:scoped?`MCP ${fixture.selection.provider_id} / http`:'MCP work / lookup'}).waitFor();
  assert.deepEqual(JSON.parse(await page.getByTestId('ahp-tool').locator('pre').innerText()),scoped?{method:'GET',url:'https://worker.tail123.ts.net/items'}:{query:'native fixture'});
  await page.getByTestId('ahp-tool-approve').waitFor();
  for(const secret of ['pipeline-private-access','pipeline-private-refresh','DUNLIN_MCP_TOKEN','bridge-scoped-private-value','browser-config-private-value'])assert.ok(!(await page.locator('body').innerText()).includes(secret));
  if(fixture.form_action) {
    await page.getByTestId('ahp-tool-approve').click();
    await page.getByTestId('ahp-input').waitFor();
    if(urlInput) {
      assert.equal(await page.getByTestId('ahp-input-url-address').innerText(),'https://review.example.test/connect?step=1');
      assert.equal(await page.getByTestId('ahp-input-url-host').innerText(),'review.example.test');
      assert.equal(await page.locator('[data-question-id]').count(),0);
      assert.equal(targetRequests.length,0);assert.equal(urlDecisions.length,0);
    } else {
    const field=name=>page.locator(`[data-question-id="${name}"]`).locator('input,select');
    assert.equal(await field('name').inputValue(),'');assert.equal(await field('active').inputValue(),'');assert.equal(await field('choice').inputValue(),'');assert.deepEqual(await field('many').evaluate(e=>Array.from(e.selectedOptions,o=>o.value)),[]);
    await field('name').fill('Reviewed');await field('count').fill('0');await field('active').selectOption('false');await field('choice').selectOption('0');await field('many').selectOption(['0','1']);await field('ratio').fill('0.5');await field('date').fill('2026-11-13');await field('email').fill('user@example.test');await field('uri').fill('https://docs.example.test/plain-text');await field('__proto__').fill('reviewed-proto');
    }
    for(const secret of ['private-mrtr-state-first','private-mrtr-state-second','private-url-state-first','private-url-state-second','pipeline-private-access','pipeline-private-refresh'])assert.ok(!(await page.locator('body').innerText()).includes(secret));
  }
  await page.evaluate(()=>scrollTo(0,0));
  const desktop=(await page.screenshot({fullPage:true})).toString('base64');
  await page.setViewportSize({width:390,height:844});
  const overflow=await page.evaluate(()=>Array.from(document.querySelectorAll('body *')).filter(element=>element.getBoundingClientRect().right>innerWidth+1).slice(0,12).map(element=>({tag:element.tagName,class:element.className,test:element.getAttribute('data-testid'),right:element.getBoundingClientRect().right})));
  assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),JSON.stringify(overflow));
  await page.evaluate(()=>scrollTo(0,0));
  const mobile=(await page.screenshot({fullPage:true})).toString('base64');
  if(fixture.form_action) {
    if(urlInput) {
      const action=fixture.form_action.slice(4);
      for(let round=1;round<=(action==='accept'?2:1);round++) {
        await page.getByTestId('ahp-input-url-address').filter({hasText:`step=${round}`}).waitFor();
        const original=await page.getByTestId('ahp-input').getAttribute('data-request-id');
        assert.equal(targetRequests.length,round-1);assert.equal(urlDecisions.length,round-1);
        const link=page.getByTestId('ahp-input-url-open');
        assert.equal(await link.getAttribute('rel'),'noopener noreferrer');
        assert.equal(await link.getAttribute('referrerpolicy'),'no-referrer');
        if(action==='accept') {
          const popupPromise=context.waitForEvent('page');await link.click();
          const popup=await popupPromise;await popup.waitForLoadState();
          assert.equal(popup.url(),`https://review.example.test/connect?step=${round}`);
          assert.equal(await popup.evaluate(()=>window.opener),null);
          assert.equal(await popup.evaluate(()=>document.referrer),'');await popup.close();
          assert.equal(await page.getByTestId('ahp-input').getAttribute('data-request-id'),original);
          assert.equal(urlDecisions.length,round-1); // Opening never submits acceptance.
        }
        await page.getByTestId(`ahp-input-${action}`).click();
      }
      await page.getByTestId('ahp-input').waitFor({state:'detached'});
      assert.equal(targetRequests.length,action==='accept'?2:0);
    } else {
    if(fixture.form_action==='accept') {
      const original=await page.getByTestId('ahp-input').getAttribute('data-request-id');
      await page.locator('[data-question-id="uri"] input').fill('not a uri');
      await page.getByTestId('ahp-input-accept').click();
      await page.getByRole('alert').filter({hasText:'MCP form response does not match its schema'}).waitFor();
      assert.equal(await page.getByTestId('ahp-input').getAttribute('data-request-id'),original);
      assert.equal(await page.locator('[data-question-id="name"] input').inputValue(),'Reviewed');
      await page.locator('[data-question-id="uri"] input').fill('https://docs.example.test/plain-text');
    }
    await page.getByTestId(`ahp-input-${fixture.form_action}`).click();
    if(fixture.form_action==='accept') {
      await page.getByText('Potwierdź drugą rundę',{exact:false}).waitFor();
      await page.locator('[data-question-id="reviewed"] select').selectOption('true');
      await page.getByTestId('ahp-input-accept').click();
    }
    await page.getByTestId('ahp-input').waitFor({state:'detached'});
    }
  } else await page.getByTestId('ahp-tool-approve').click();
  await page.getByTestId('ahp-transcript').getByText('remembered:remote-ui-context',{exact:true}).waitFor();
  await page.getByTestId('ahp-permission').waitFor({state:'detached'});
  await page.getByTestId('ahp-cancel-turn').waitFor({state:'detached'});
  assert.deepEqual(errors,[]);
  await context.close();
  process.stdout.write(JSON.stringify({status:'passed',session_id:id,epoch:accepted.session.epoch,desktop,mobile,scopedDesktop,scopedMobile})+'\n');
} catch(error) {if(page)console.error('Visible UI:',(await page.locator('body').innerText()).slice(0,6000));throw error;}
finally {await browser.close();}
