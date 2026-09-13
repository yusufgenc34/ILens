import {expect, test, type Page} from '@playwright/test'
import path from 'node:path'
import {execFileSync} from 'node:child_process'

async function open(page: Page, fixture: string) {
  await page.goto('/')
  await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(path.resolve(`samples/fixtures/ILens.${fixture}.dll`))
  await expect(page.getByLabel('Primary assembly')).toHaveValue('1')
}
async function editMethod(page: Page, name: string) {
  await page.keyboard.press('ControlOrMeta+k')
  await page.getByLabel('Search members and strings').fill(name)
  await page.locator('.search-results > button').filter({has: page.locator('strong', {hasText: new RegExp(`^${name}$`)})}).click()
  await page.getByRole('tab', {name: 'IL', exact: true}).click()
  await page.getByRole('button', {name: 'Edit IL', exact: true}).click()
  return page.getByRole('dialog', {name: 'Edit method IL', exact: true})
}
function verify(file: string) {
  // Static verification of repository-owned output only; never run the assembly.
  const runtimes = execFileSync('dotnet', ['--list-runtimes'], {encoding: 'utf8'})
  const runtime = runtimes.split('\n').find(line => line.startsWith('Microsoft.NETCore.App 10.'))?.match(/^Microsoft.NETCore.App (\S+) \[(.+)\]$/)
  expect(runtime).toBeTruthy()
  const output = execFileSync('dotnet', ['tool', 'run', 'ilverify', file, '-r', path.join(runtime![2], runtime![1], '*.dll'), '-r', path.resolve('samples/fixtures/ILens.Dependency.dll'), '-s', 'System.Private.CoreLib'], {encoding: 'utf8', timeout: 30_000})
  expect(output).toContain('Verified')
}
test('MakeArray edits reject a wrong element opcode and produce a verified DLL with the changed constant', async ({page}, testInfo) => {
  await open(page, 'Patterns')
  const dialog = await editMethod(page, 'MakeArray')
  await expect(dialog.getByLabel('Opcode IL_0000', {exact: true})).toBeEnabled()
  const store = dialog.locator('input[aria-label^="Opcode"][value="stelem.i4"]').first()
  await store.fill('stelem.ref')
  await dialog.getByRole('button', {name: 'Validate and apply'}).click()
  await expect(dialog.getByRole('alert')).toContainText('array element type')
  await dialog.getByLabel('Undo IL change').click()
  await dialog.locator('input[aria-label^="Operand"][value="42"]').fill('43')
  await dialog.getByRole('button', {name: 'Validate and apply'}).click()
  await expect(dialog).toContainText('Validation passed.')
  await dialog.getByRole('button', {name: 'Validated IL', exact: true}).click()
  await expect(dialog.getByLabel('Validated method IL')).toContainText('43')
  await dialog.locator('.cm-line').filter({hasText: /ldc\.i4\.s\s+43/}).scrollIntoViewIfNeeded()
  await page.screenshot({path: testInfo.outputPath('array-il-editor.png'), animations: 'disabled'})
  await dialog.getByRole('button', {name: 'Export modified assembly'}).click()
  const download = page.waitForEvent('download')
  await page.getByRole('button', {name: 'Download modified assembly'}).click()
  const file = testInfo.outputPath('ILens.Patterns.modified.dll'); await (await download).saveAs(file)
  verify(file)
  await page.getByRole('button', {name: 'Close export'}).click()
  await page.getByRole('button', {name: 'Close workspace', exact: true}).click()
  await page.getByLabel('Open assembly files').setInputFiles(file)
  await expect(page.getByLabel('Primary assembly')).toHaveValue('1')
  await page.keyboard.press('ControlOrMeta+k'); await page.getByLabel('Search members and strings').fill('MakeArray')
  await page.getByRole('button', {name: /MakeArray.*Calculator.MakeArray/}).click()
  await expect(page.getByLabel('C# code viewer')).toContainText('43')
})
test('array, object, constructor-call and instance edits pass independent whole-assembly ILVerify', async ({page}, testInfo) => {
  test.setTimeout(120_000)
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message))
  await open(page, 'Editing')
  // Each method gets a real encoded change, exercising growing headers/bodies,
  // multiple overlays, typed merges, widths, boxing and member dispatch.
  const names = ['Create', 'Call', 'InterfaceCall', 'DerivedCall', 'SetProperty', 'StringLength', 'ThrowException', 'RuntimeType', 'ToObject', 'AsCounter', 'CastCounter', 'Strings', 'Objects', 'Jagged', 'Counters', 'FirstCounter', 'Covariant', 'Select', 'SelectKinds', 'NotNull', 'Length', 'ByteAt', 'ShortAt', 'LongAt', 'FloatAt', 'DoubleAt', 'NativeAt', 'StoreByte', 'StoreShort', 'StoreLong', 'StoreFloat', 'StoreDouble', 'StoreNative', 'Increase', 'Rename']
  for (const name of names) {
    const dialog = await editMethod(page, name)
    await dialog.getByLabel('Insert before IL_0000', {exact: true}).click()
    if (name === 'Increase') await dialog.locator('input[aria-label^="Opcode"][value="add"]').fill('sub')
    await dialog.getByRole('button', {name: 'Validate and apply'}).click()
    await expect(dialog).toContainText('Validation passed.')
    await dialog.getByRole('button', {name: 'Close IL editor'}).click()
  }
  await page.getByRole('menuitem', {name: 'File', exact: true}).click()
  await page.getByRole('menuitem', {name: 'Save Module…', exact: true}).click()
  const download = page.waitForEvent('download')
  await page.getByRole('button', {name: 'Download modified assembly'}).click()
  const file = testInfo.outputPath('ILens.Editing.modified.dll'); await (await download).saveAs(file)
  verify(file)
  await page.getByRole('button', {name: 'Close export'}).click()
  await page.getByRole('button', {name: 'Close workspace', exact: true}).click()
  await page.getByLabel('Open assembly files').setInputFiles(file)
  await expect(page.getByLabel('Primary assembly')).toHaveValue('1')
  const dialog = await editMethod(page, 'Increase')
  await expect(dialog.locator('input[aria-label^="Opcode"][value="sub"]')).toHaveCount(1)
  await expect(dialog.getByLabel('Opcode IL_0000', {exact: true})).toHaveValue('nop')
  expect(errors).toEqual([])
})
