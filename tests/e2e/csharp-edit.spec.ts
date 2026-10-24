import {expect, test} from './test'
import path from 'node:path'
import fs from 'node:fs'
import {execFileSync} from 'node:child_process'

// Microsoft's reference assemblies (metadata only) from the installed SDK, so the test needs no network.
function referencePack() {
  const root = path.dirname(fs.realpathSync(execFileSync('which', ['dotnet'], {encoding: 'utf8'}).trim()))
  const packs = path.join(root, 'packs', 'Microsoft.NETCore.App.Ref')
  const version = fs.readdirSync(packs).filter(v => v.startsWith('10.')).sort().at(-1)!
  return path.join(packs, version, 'ref', 'net10.0')
}
test('Edit Method (C#) compiles in the browser, merges the body and produces a verified DLL', async ({page}, testInfo) => {
  test.skip(!fs.existsSync('public/compiler/_framework/dotnet.js'), 'Run npm run build:compiler first.')
  test.setTimeout(180_000)
  await page.goto('./')
  await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(path.resolve('samples/fixtures/ILens.Editing.dll'))
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id', '1')
  const refs = referencePack()
  await page.getByLabel('Add dependency files').setInputFiles(['System.Runtime.dll', 'System.Console.dll'].map(f => path.join(refs, f)))
  await page.keyboard.press('ControlOrMeta+k')
  await page.getByLabel('Search members and strings').fill('Increase')
  await page.locator('.search-results > button').filter({has: page.locator('strong', {hasText: /^Increase$/})}).first().click()
  await page.getByRole('treeitem', {name: /^Increase/}).first().click({button: 'right'})
  await page.getByRole('menuitem', {name: 'Edit Method (C#)…', exact: true}).click()
  const dialog = page.getByRole('dialog', {name: 'Edit Method (C#)'})
  const editor = dialog.getByLabel('C# method source')
  await expect(editor).toContainText('Increase(int amount)')
  // Introduces a new member reference (Console.WriteLine) and a new string literal.
  await editor.click()
  await page.keyboard.press('ControlOrMeta+a')
  await page.keyboard.insertText('public int Increase(int amount)\n{\nint doubled = amount * 2;\nSystem.Console.WriteLine("adding " + doubled);\nValue += doubled;\nreturn Value;\n}')
  await dialog.getByRole('button', {name: 'Compile and apply'}).click()
  await expect(dialog).toContainText('Applied.', {timeout: 120_000})
  await page.screenshot({path: testInfo.outputPath('csharp-edit.png'), animations: 'disabled'})
  await dialog.getByRole('button', {name: 'Close', exact: true}).click()
  await page.getByRole('menuitem', {name: 'File', exact: true}).click()
  await page.getByRole('menuitem', {name: /^Save Module/}).click()
  const download = page.waitForEvent('download')
  await page.getByRole('button', {name: 'Download modified assembly'}).click()
  const file = testInfo.outputPath('ILens.Editing.csharp.dll'); await (await download).saveAs(file)
  // Static verification only; the assembly is never run.
  const runtimes = execFileSync('dotnet', ['--list-runtimes'], {encoding: 'utf8'})
  const runtime = runtimes.split('\n').find(line => line.startsWith('Microsoft.NETCore.App 10.'))!.match(/^Microsoft.NETCore.App (\S+) \[(.+)\]$/)!
  expect(execFileSync('dotnet', ['tool', 'run', 'ilverify', file, '-r', path.join(runtime[2], runtime[1], '*.dll'), '-s', 'System.Private.CoreLib'], {encoding: 'utf8', timeout: 30_000})).toContain('Verified')
  await page.getByRole('button', {name: 'Close export'}).click()
  await page.getByRole('button', {name: 'Close workspace', exact: true}).click()
  await page.getByLabel('Open assembly files').setInputFiles(file)
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id', '1')
  await page.keyboard.press('ControlOrMeta+k')
  await page.getByLabel('Search members and strings').fill('Increase')
  await page.locator('.search-results > button').filter({has: page.locator('strong', {hasText: /^Increase$/})}).first().click()
  await expect(page.getByLabel('C# code viewer')).toContainText('Console.WriteLine("adding " + ')
})
