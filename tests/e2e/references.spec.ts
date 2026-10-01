import {expect,test,type Page} from './test'
import {execFileSync} from 'node:child_process'
import {readFile,readdir} from 'node:fs/promises'
import path from 'node:path'
import {createZipArchive} from '../../src/lib/zip'
test.use({frameworkDownloads:true})
const packageURL='https://api.nuget.org/v3-flatcontainer/microsoft.netcore.app.ref/10.0.0/microsoft.netcore.app.ref.10.0.0.nupkg'
async function sdkPackage() {
  // Reuse genuine SDK reference assemblies without distributing third-party DLL fixtures.
  const sdk=/^10\.[^\n[]+\[(.+)\]/m.exec(execFileSync('dotnet',['--list-sdks'],{encoding:'utf8'}))?.[1]
  if(!sdk) throw new Error('The reference integration tests require the .NET 10 SDK.')
  const root=path.resolve(sdk,'..','packs','Microsoft.NETCore.App.Ref')
  const version=(await readdir(root)).filter(v=>v.startsWith('10.')).sort((a,b)=>b.localeCompare(a,undefined,{numeric:true}))[0]
  const zip=createZipArchive()
  const directory=path.join(root,version,'ref','net10.0')
  for(const name of (await readdir(directory)).filter(n=>n.endsWith('.dll'))) zip.add(`ref/net10.0/${name}`,new Uint8Array(await readFile(path.join(directory,name))))
  return Buffer.from(zip.finish())
}
async function open(page:Page,filename='ILens.Workspace.dll') {
  await page.goto('./');await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(path.resolve('samples/fixtures',filename))
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id','1')
  if(filename==='ILens.Workspace.dll') await page.getByRole('treeitem',{name:'References',exact:true}).locator('.disclosure').click()
}
const reference=(page:Page,name='System.Runtime')=>page.getByRole('treeitem',{name,exact:true}).filter({has:page.getByLabel('Unresolved reference',{exact:true})})
async function settings(page:Page) {
  await page.getByRole('menuitem',{name:'Edit',exact:true}).click()
  await page.getByRole('menuitem',{name:'Settings…',exact:true}).click()
}
test('framework references download automatically on open and remain alongside the original assembly',async({page},testInfo)=>{
  const errors:string[]=[],requests:string[]=[],posts:string[]=[]
  page.on('pageerror',error=>errors.push(error.message))
  page.on('request',request=>{if(request.url().startsWith('https://api.nuget.org/'))requests.push(request.url());if(request.method()==='POST')posts.push(request.url())})
  if(!process.env.ILENS_LIVE_NUGET) {
    const bytes=await sdkPackage()
    await page.route(packageURL,route=>route.fulfill({body:bytes,contentType:'application/octet-stream',headers:{'access-control-allow-origin':'*'}}))
  }
  await open(page)
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('2',{timeout:25000})
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id','1')
  await expect(page.locator('[role="treeitem"][aria-level="1"]')).toHaveCount(2)
  await expect(page.getByRole('treeitem',{name:'ILens.Workspace',exact:true})).toBeVisible()
  await expect(page.locator('[role="treeitem"][aria-level="1"]',{hasText:'System.Runtime'})).toBeVisible()
  await expect(page.getByRole('treeitem',{name:'System.Runtime',exact:true}).filter({has:page.getByLabel('Resolved reference',{exact:true})})).toBeVisible()
  expect(requests).toEqual([packageURL]);expect(posts).toEqual([])
  await expect(page.locator('.workspace')).toHaveCSS('display','grid')
  await page.screenshot({path:testInfo.outputPath('framework-explorer.png'),animations:'disabled'})
  // The reference opens the existing root instead of loading another copy.
  await page.getByRole('treeitem',{name:'System.Runtime',exact:true}).filter({has:page.getByLabel('Resolved reference',{exact:true})}).dblclick()
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id','2')
  await expect(page.getByRole('note')).toContainText('API metadata only')
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('2')
  expect(requests).toHaveLength(1);expect(errors).toEqual([])
})
test('automatic downloads can be disabled before opening and private references never become remote queries',async({page})=>{
  const requests:string[]=[]
  page.on('request',request=>{if(request.url().startsWith('https://api.nuget.org/')) requests.push(request.url())})
  await page.addInitScript(()=>localStorage.setItem('ilens:framework-downloads','false'))
  await open(page);await reference(page).dblclick()
  await expect(page.getByRole('alert')).toContainText('unavailable locally')
  expect(requests).toEqual([])
  const bytes=await sdkPackage()
  await page.route(packageURL,route=>route.fulfill({body:bytes,headers:{'access-control-allow-origin':'*'}}))
  await settings(page);await page.getByLabel('Download .NET references automatically').check();await page.getByRole('button',{name:'Close settings'}).click()
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('2')
  await expect(page.getByRole('menuitem',{name:'File',exact:true})).toBeEnabled()
  await expect(page.getByRole('button',{name:'Stop analysis'})).toHaveCount(0)
  await reference(page,'ILens.WorkspaceDependency').dblclick()
  await expect(page.getByRole('alert')).toContainText('No automatic package source')
  expect(requests).toEqual([packageURL])
})
test('a failed download preserves the workspace and can be retried',async({page})=>{
  await page.route(packageURL,route=>route.fulfill({status:503,headers:{'access-control-allow-origin':'*'}}))
  await open(page)
  await expect(page.getByRole('alert')).toContainText('dependencies could not be resolved')
  await expect(page.getByRole('alert').locator('code')).toContainText('HTTP 503')
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id','1')
  const bytes=await sdkPackage()
  await page.route(packageURL,route=>route.fulfill({body:bytes,headers:{'access-control-allow-origin':'*'}}))
  await page.getByRole('menuitem',{name:'File',exact:true}).click()
  await page.getByRole('menuitem',{name:'Retry Missing Dependencies',exact:true}).click()
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('2')
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id','1')
})
test('closing the workspace cancels a download and a delayed result cannot reopen it',async({page})=>{
  let release!:()=>void
  const paused=new Promise<void>(resolve=>{release=resolve})
  await page.route(packageURL,async route=>{await paused;await route.abort().catch(()=>{})})
  const request=page.waitForRequest(packageURL)
  await open(page);await request
  await page.getByRole('button',{name:'Close workspace'}).click()
  release()
  await expect(page.getByText('No assemblies loaded',{exact:true})).toBeVisible()
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('0')
  await expect(page.getByRole('alert')).toHaveCount(0)
})
test('C# reference navigation uses the automatically loaded external definition and labels placeholder code',async({page})=>{
  const bytes=await sdkPackage()
  await page.route(packageURL,route=>route.fulfill({body:bytes,headers:{'access-control-allow-origin':'*'}}))
  await open(page)
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('2')
  await expect(page.getByRole('button',{name:'Stop analysis'})).toHaveCount(0)
  await page.keyboard.press('ControlOrMeta+k');await page.getByLabel('Search members and strings').fill('UnicodeBeforeCall')
  await page.locator('.search-results > button').filter({has:page.locator('strong',{hasText:/^UnicodeBeforeCall$/})}).click()
  const viewer=page.getByLabel('C# code viewer')
  await expect(viewer).toContainText('Concat')
  await viewer.locator('.cursor-pointer').filter({hasText:/^Concat$/}).click({modifiers:['ControlOrMeta']})
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id','2')
  await expect(viewer).toContainText('Reference assembly')
  await expect(viewer).not.toContainText('throw null')
  await page.getByRole('tab',{name:'IL',exact:true}).click()
  await expect(page.getByLabel('IL code viewer')).toContainText('placeholder instructions')
})
test('navigation made during a download is preserved while the new root is retained',async({page})=>{
  const bytes=await sdkPackage();let release!:()=>void
  const paused=new Promise<void>(resolve=>{release=resolve})
  await page.route(packageURL,async route=>{await paused;await route.fulfill({body:bytes,headers:{'access-control-allow-origin':'*'}})})
  const request=page.waitForRequest(packageURL)
  await open(page);await request
  await page.getByRole('treeitem',{name:'ILens.Samples',exact:true}).click()
  await page.getByRole('treeitem',{name:'WorkspacePatterns',exact:true}).locator('.disclosure').click()
  await page.getByRole('treeitem',{name:/^DependencyProperty\(/}).click()
  release()
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('2')
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id','1')
  await expect(page.getByRole('treeitem',{name:/^DependencyProperty\(/})).toHaveAttribute('aria-selected','true')
  await expect(page.getByLabel('C# code viewer')).toContainText('DependencyProperty')
})

test('a managed EXE resolves its dependencies without clicking any reference',async({page},testInfo)=>{
  const bytes=await sdkPackage(),requests:string[]=[]
  page.on('request',r=>{if(r.url().startsWith('https://api.nuget.org/')) requests.push(r.url())})
  if(!process.env.ILENS_LIVE_NUGET) await page.route(packageURL,route=>route.fulfill({body:bytes,headers:{'access-control-allow-origin':'*'}}))
  await open(page,'ILens.AutomaticDependencies.exe')
  await expect(page.getByRole('button',{name:'Stop analysis'})).toHaveCount(0,{timeout:25000})
  for(const name of ['System.Runtime','System.Collections','System.Console']) await expect(page.locator('[role="treeitem"][aria-level="1"]',{hasText:name})).toBeVisible()
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id','1')
  expect(requests).toEqual([packageURL])
  await expect(page.getByRole('alert')).toHaveCount(0)
  await expect(page.locator('.workspace')).toHaveCSS('display','grid')
  await page.screenshot({path:testInfo.outputPath('automatic-exe-dependencies.png'),animations:'disabled'})
})
