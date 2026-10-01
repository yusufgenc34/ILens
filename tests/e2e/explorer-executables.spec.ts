import {test,expect} from './test'
import {readFile,mkdir,writeFile} from 'node:fs/promises'
import {execFileSync} from 'node:child_process'
import path from 'node:path'
const patterns=path.resolve('samples/fixtures/ILens.Patterns.dll')
async function nativeEnvelope() {
  // Structural native-PE fixture: remove the CLI directory from an owned PE.
  // This does not claim to be an apphost or execute any code.
  const buffer=await readFile('samples/fixtures/ILens.AutomaticDependencies.exe')
  const pe=buffer.readUInt32LE(60),directory=pe+24+96+14*8
  buffer.fill(0,directory,directory+8)
  return {name:'ILens.AutomaticDependencies.exe',mimeType:'application/octet-stream',buffer}
}
test('Explorer follows assembly/module/namespace/type/member and property accessor hierarchy',async({page},info)=>{
  await page.goto('./');await page.getByLabel('Open assembly files').setInputFiles(patterns)
  await expect(page.getByRole('treeitem',{name:'ILens.Patterns',exact:true})).toHaveAttribute('aria-level','1')
  await expect(page.getByRole('treeitem',{name:'ILens.Patterns.dll',exact:true})).toHaveAttribute('aria-level','2')
  await page.getByRole('treeitem',{name:'ILens.Samples',exact:true}).click()
  await page.getByRole('treeitem',{name:'Calculator',exact:true}).locator('.disclosure').click()
  const add=page.getByRole('treeitem',{name:'Add(int, int) : int',exact:true})
  await expect(add).toHaveAttribute('aria-level','5');await add.click()
  await expect(page.getByLabel('C# code viewer')).toContainText('return (left + right)')
  await expect(page.locator('.cm-scroller')).toHaveJSProperty('scrollLeft',0)
  await page.screenshot({path:info.outputPath('assembly-explorer-hierarchy.png'),animations:'disabled'})
  await expect(page.getByRole('treeitem',{name:/^(Methods|Fields|Properties|Events|Constructors)$/})).toHaveCount(0)
  const property=page.getByRole('treeitem',{name:'Value : int',exact:true})
  await property.locator('.disclosure').click()
  const getter=page.getByRole('treeitem',{name:'get_Value() : int',exact:true})
  await expect(getter).toHaveAttribute('aria-level','6');await getter.click()
  await expect(page.getByLabel('C# code viewer')).toContainText('get_Value')
  await page.keyboard.press('ArrowLeft')
  await expect(property).toHaveAttribute('aria-selected','true')
  await page.keyboard.press('ArrowLeft')
  await expect(property).toHaveAttribute('aria-expanded','false')
  await expect(getter).toHaveCount(0)
  await expect(page.getByLabel('Primary assembly')).toHaveCount(0)
  await page.screenshot({path:info.outputPath('assembly-explorer-dark.png'),animations:'disabled'})
  await page.getByRole('menuitem',{name:'View',exact:true}).click()
  await page.getByRole('menuitemcheckbox',{name:'Light Appearance'}).click()
  await page.screenshot({path:info.outputPath('assembly-explorer-light.png'),animations:'disabled'})
})
test('EXE with no CLR metadata opens a matching selected DLL and reports an actionable error otherwise',async({page})=>{
  await page.goto('./')
  const exe=await nativeEnvelope(),dll={name:'ILens.AutomaticDependencies.dll',mimeType:'application/octet-stream',buffer:await readFile('samples/fixtures/ILens.AutomaticDependencies.exe')}
  await page.getByLabel('Open assembly files').setInputFiles([exe,dll])
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('1')
  await expect(page.getByRole('alert')).toHaveCount(0)
  await expect(page.getByRole('treeitem',{name:dll.name,exact:true})).toBeVisible()
  await expect(page.locator('.status-message')).toContainText('companion DLLs opened')
  await page.getByRole('button',{name:'Close workspace'}).click()
  await page.getByLabel('Open assembly files').setInputFiles(exe)
  await expect(page.getByRole('alert')).toContainText('Open ILens.AutomaticDependencies.dll or select its application folder')
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('0')
})
test('choosing the application folder retries a pending EXE and opens its companion',async({page},info)=>{
  await page.goto('./');await page.getByLabel('Open assembly files').setInputFiles(await nativeEnvelope())
  await expect(page.getByRole('alert')).toContainText('has no CLR metadata')
  const folder=info.outputPath('application');await mkdir(folder,{recursive:true})
  await writeFile(path.join(folder,'ILens.AutomaticDependencies.dll'),await readFile('samples/fixtures/ILens.AutomaticDependencies.exe'))
  await page.getByLabel('Dependency folder',{exact:true}).setInputFiles(folder)
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('1')
  await expect(page.getByRole('treeitem',{name:'ILens.AutomaticDependencies',exact:true})).toBeVisible()
  await expect(page.getByRole('alert')).toHaveCount(0)
})
test('a genuine SDK ReadyToRun assembly exposes its preserved IL with a read-only warning',async({page})=>{
  const runtimes=execFileSync('dotnet',['--list-runtimes'],{encoding:'utf8'})
  const runtime=runtimes.split('\n').find(line=>line.startsWith('Microsoft.NETCore.App 10.'))!.match(/^Microsoft.NETCore.App (\S+) \[(.+)\]$/)!
  await page.goto('./')
  await page.getByLabel('Open assembly files').setInputFiles(path.join(runtime[2],runtime[1],'System.Console.dll'))
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('1')
  await page.getByLabel('Assembly warnings').locator('summary').click()
  await expect(page.getByLabel('Assembly warnings')).toContainText('ReadyToRun image')
  await page.keyboard.press('ControlOrMeta+k');await page.getByLabel('Search members and strings').fill('get_IsOutputRedirected')
  await page.locator('.search-results > button').filter({has:page.locator('strong',{hasText:/^get_IsOutputRedirected$/})}).click()
  await page.getByRole('tab',{name:'IL',exact:true}).click()
  await expect(page.getByLabel('IL code viewer')).toContainText('ret')
  await page.getByRole('button',{name:'Edit IL',exact:true}).click()
  await expect(page.getByRole('dialog',{name:'Edit method IL'})).toContainText('ReadyToRun images are read-only')
})
