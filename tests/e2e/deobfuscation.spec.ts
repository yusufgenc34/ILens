import {test,expect,type Page} from './test'
import {readFile} from 'node:fs/promises'
import {execFileSync} from 'node:child_process'
import path from 'node:path'
const fixture=path.resolve('samples/fixtures/ILens.Deobfuscation.dll')
async function open(page:Page){
  await page.goto('./');await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(fixture)
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('1')
  await page.getByRole('menuitem',{name:'Extras',exact:true}).click()
  await page.getByRole('menuitem',{name:'Deobfuscation Options…',exact:true}).click()
}
async function selectMessage(page:Page){
  await page.keyboard.press('ControlOrMeta+k');await page.getByLabel('Search members and strings').fill('Message')
  await page.locator('.search-results > button').filter({has:page.locator('strong',{hasText:/^Message$/})}).click()
}
test('real worker cleans a pure string helper, verifies/downloads the PE and opens an isolated copy',async({page},info)=>{
  const sent:string[]=[];page.on('request',r=>{if(r.method()!=='GET')sent.push(r.url())})
  await open(page)
  const dialog=page.getByRole('dialog',{name:'Deobfuscate assembly',exact:true})
  await expect(dialog.getByRole('checkbox',{name:/Evaluate pure string/})).not.toBeChecked()
  await dialog.getByRole('checkbox',{name:/Evaluate pure string/}).check()
  await dialog.getByRole('button',{name:'Run deobfuscation'}).click()
  const report=dialog.getByRole('region',{name:'Deobfuscation report'})
  await expect(report).toContainText('2 strings',{timeout:20000})
  await expect(report).toContainText('partial')
  await expect(dialog.getByRole('button',{name:'Done',exact:true})).toBeInViewport({ratio:1})
  expect(await dialog.evaluate(e=>getComputedStyle(e).backgroundColor)).not.toBe('rgba(0, 0, 0, 0)')
  await page.screenshot({path:info.outputPath('deobfuscation-report.png'),animations:'disabled'})
  const jsonDownload=page.waitForEvent('download');await dialog.getByRole('button',{name:'Download report',exact:true}).click()
  const json=JSON.parse(await readFile((await (await jsonDownload).path())!,'utf8'))
  expect(json.methods.flatMap((m:{reasons:string[]})=>m.reasons).join('\n')).toMatch(/initializer/)
  const download=page.waitForEvent('download');await dialog.getByRole('button',{name:'Download cleaned assembly'}).click()
  const result=await download;expect(result.suggestedFilename()).toBe('ILens_Deobfuscation.cleaned.dll')
  const saved=info.outputPath(result.suggestedFilename());await result.saveAs(saved)
  const data=await readFile(saved);expect(data.equals(await readFile(fixture))).toBe(false)
  if(info.project.name==='chromium'){
    const runtimes=execFileSync('dotnet',['--list-runtimes'],{encoding:'utf8'})
    const runtime=runtimes.split('\n').find(l=>l.startsWith('Microsoft.NETCore.App 10.'))!.match(/^Microsoft.NETCore.App (\S+) \[(.+)\]$/)!
    const verified=execFileSync('dotnet',['tool','run','ilverify',saved,'-r',path.join(runtime[2],runtime[1],'*.dll'),'-s','System.Private.CoreLib'],{encoding:'utf8',timeout:30000})
    expect(verified).toContain('Verified')
  }
  await dialog.getByRole('button',{name:'Open copy in workspace'}).click()
  await expect(dialog).toHaveCount(0);await expect(page.getByLabel('Loaded assemblies')).toHaveText('2')
  await expect(page.getByRole('treeitem',{name:/ILens.Deobfuscation \[cleaned copy #2\]/})).toBeVisible()
  await selectMessage(page);await expect(page.getByLabel('C# code viewer')).toContainText('Hello ILens')
  await page.getByRole('tab',{name:'IL',exact:true}).click();await expect(page.getByLabel('IL code viewer')).not.toContainText('Decode')
  await page.getByRole('tree',{name:'Assembly declarations'}).focus();await page.keyboard.press('Home')
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id','1')
  await selectMessage(page);await page.getByRole('tab',{name:'IL',exact:true}).click();await expect(page.getByLabel('IL code viewer')).toContainText('Decode')
  expect(sent).toEqual([])
  await page.getByRole('button',{name:'Close workspace',exact:true}).click()
  await page.getByLabel('Open assembly files').setInputFiles(saved)
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('1');await selectMessage(page)
  await expect(page.getByLabel('C# code viewer')).toContainText('Hello ILens')
})
test('cancel releases a real in-flight job and a new job produces byte-identical no-op output',async({page})=>{
  await page.addInitScript(()=>{
    const post=Worker.prototype.postMessage
    Worker.prototype.postMessage=function(this:Worker,message:unknown,options?:Transferable[]|StructuredSerializeOptions){
      const send=()=>post.call(this,message,Array.isArray(options)?{transfer:options}:options)
      if(message&&typeof message==='object'&&'op' in message&&message.op==='deobfuscateStep')setTimeout(send,40);else send()
    }
  })
  await open(page);const dialog=page.getByRole('dialog',{name:'Deobfuscate assembly',exact:true})
  // These three legitimately change this fixture; disable them for a true no-op.
  for(const pass of [/Unwrap proxy\/indirect calls/,/Straighten scrambled control flow/,/Answer debugger checks/])
    await dialog.getByRole('checkbox',{name:pass}).uncheck()
  await dialog.getByRole('button',{name:'Run deobfuscation'}).click()
  await expect(dialog.locator('progress')).toBeVisible()
  await dialog.getByRole('button',{name:'Cancel deobfuscation'}).click()
  await expect(dialog).toContainText('Pending changes were discarded')
  await expect(dialog.getByRole('button',{name:/Download.*assembly/})).toHaveCount(0)
  await dialog.getByRole('button',{name:'Run deobfuscation'}).click()
  await expect(dialog).toContainText('No supported changes found.',{timeout:20000})
  const event=page.waitForEvent('download');await dialog.getByRole('button',{name:'Download unchanged assembly'}).click()
  expect(await readFile((await (await event).path())!)).toEqual(await readFile(fixture))
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('1')
})
test('Fix it on the obfuscation warning renames hidden names and opens the cleaned copy',async({page},info)=>{
  // Owned stand-in for invisible-name protection: same-length control characters.
  let bytes=await readFile(fixture)
  ;['Patterns','WithInitializer','Decode','Fresh'].forEach((name,k)=>{
    const at=bytes.indexOf(Buffer.from(`\0${name}\0`,'latin1'))+1;expect(at).toBeGreaterThan(0)
    bytes=Buffer.concat([bytes.subarray(0,at),Buffer.from([k+1,...Array(name.length-1).fill(0x1e)]),bytes.subarray(at+name.length)])
  })
  const hidden=info.outputPath('ILens.Hidden.dll');await import('node:fs/promises').then(fs=>fs.writeFile(hidden,bytes))
  await page.goto('./');await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(hidden)
  const warnings=page.getByLabel('Assembly warnings')
  await expect(warnings).toContainText('Possible obfuscation')
  await warnings.getByRole('button',{name:'Fix it'}).click()
  await expect(page.getByRole('dialog',{name:'Deobfuscate assembly',exact:true})).toHaveCount(0,{timeout:30000})
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('2')
  await expect(page.getByRole('treeitem',{name:/cleaned copy/})).toBeVisible()
  // The copy only carries its provenance note, no obfuscation finding or Fix it.
  await expect(page.getByLabel('Assembly warnings')).not.toContainText('Possible obfuscation')
  await expect(page.getByLabel('Assembly warnings').getByRole('button',{name:'Fix it'})).toHaveCount(0)
  await page.keyboard.press('ControlOrMeta+k');await page.getByLabel('Search members and strings').fill('smethod_')
  await expect(page.locator('.search-results > button').first()).toBeVisible()
})
test.describe('while dependencies download',()=>{
  test.use({frameworkDownloads:true})
  test('Fix it stays available, queues, and starts once loading finishes',async({page},info)=>{
    let bytes=await readFile(fixture)
    const at=bytes.indexOf(Buffer.from('\0Patterns\0','latin1'))+1
    bytes=Buffer.concat([bytes.subarray(0,at),Buffer.from([1,...Array(7).fill(0x1e)]),bytes.subarray(at+8)])
    const hidden=info.outputPath('ILens.Hidden.dll');await import('node:fs/promises').then(fs=>fs.writeFile(hidden,bytes))
    // Hold the framework download so the workspace is still loading when Fix it is pressed.
    let release=()=>{}
    const held=new Promise<void>(resolve=>{release=resolve})
    await page.route('https://api.nuget.org/**',async route=>{await held;await route.fulfill({status:404,body:''})})
    await page.goto('./');await expect(page.getByLabel('Open assembly files')).toBeEnabled()
    await page.getByLabel('Open assembly files').setInputFiles(hidden)
    const warnings=page.getByLabel('Assembly warnings')
    await warnings.getByRole('button',{name:'Fix it'}).click()
    await expect(warnings.getByRole('button',{name:/waiting for dependencies/})).toBeDisabled()
    release()
    await expect(page.getByRole('treeitem',{name:/cleaned copy/})).toBeVisible({timeout:30000})
  })
})
test('proxy call unwrapping redirects a forwarder to its target in the cleaned copy',async({page})=>{
  await page.goto('./')
  await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles('samples/fixtures/ILens.Proxies.dll')
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('1')
  await page.getByRole('menuitem',{name:'Extras',exact:true}).click()
  await page.getByRole('menuitem',{name:'Deobfuscation Options…',exact:true}).click()
  const dialog=page.getByRole('dialog',{name:'Deobfuscate assembly',exact:true})
  await expect(dialog.getByRole('checkbox',{name:/Unwrap proxy\/indirect calls/})).toBeChecked()
  await dialog.getByRole('button',{name:'Run deobfuscation'}).click()
  await expect(dialog.getByRole('region',{name:'Deobfuscation report'})).toContainText(/proxy calls/,{timeout:20000})
  await dialog.getByRole('button',{name:'Open copy in workspace'}).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('2')
  // UseStatic now calls Real directly instead of the Forward proxy.
  await page.keyboard.press('ControlOrMeta+k')
  await page.getByLabel('Search members and strings').fill('UseStatic')
  await page.locator('.search-results > button').filter({has:page.locator('strong',{hasText:/^UseStatic$/})}).click()
  const viewer=page.getByLabel('C# code viewer')
  await expect(viewer).toContainText('Real(2, 3)')
  await expect(viewer).not.toContainText('Forward(2, 3)')
})
test('control-flow straightening and debugger answers are offered and reported',async({page})=>{
  await open(page)
  const dialog=page.getByRole('dialog',{name:'Deobfuscate assembly',exact:true})
  // Both are on by default: they are the two passes a protected file usually needs.
  await expect(dialog.getByRole('checkbox',{name:/Straighten scrambled control flow/})).toBeChecked()
  await expect(dialog.getByRole('checkbox',{name:/Answer debugger checks/})).toBeChecked()
  for(const other of [/Decode protected resource strings/,/Rename obfuscated symbols/,/Fold int32 constants/,/Simplify constant branches/,/Unwrap proxy/])
    await dialog.getByRole('checkbox',{name:other}).uncheck()
  await dialog.getByRole('button',{name:'Run deobfuscation'}).click()
  const report=dialog.getByRole('region',{name:'Deobfuscation report'})
  await expect(report).toContainText('3 control-flow steps',{timeout:20000})
  await expect(report).toContainText('2 debugger checks')
  await expect(report).toContainText('no debugger is attached')
})
