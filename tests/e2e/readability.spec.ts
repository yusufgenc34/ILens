import {test, expect} from '@playwright/test'
import fs from 'node:fs/promises'
import path from 'node:path'
import {execFileSync} from 'node:child_process'

test('worker output simplifies a message array and saves C# that compiles', async ({page}, testInfo) => {
  test.setTimeout(90_000)
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto('./')
  await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(path.resolve('samples/fixtures/ILens.Readability.dll'))
  await expect(page.getByLabel('Primary assembly')).toHaveValue('1')
  await page.keyboard.press('ControlOrMeta+k')
  await page.getByLabel('Search members and strings').fill('BuildMessage')
  await page.getByRole('button', {name: /BuildMessage.*FormValues.BuildMessage/}).click()
  const viewer = page.getByLabel('C# code viewer')
  await expect(viewer).toContainText('new System.String[12]')
  await expect(viewer).toContainText('System.Convert.ToString(this.Checked)')
  await expect(viewer).not.toContainText('v_')
  await expect(page.locator('.workspace')).toHaveCSS('display', 'grid')
  await page.screenshot({path: testInfo.outputPath('readable-message.png'), fullPage: true})
  await page.getByRole('menuitem', {name: 'File', exact: true}).click()
  await page.getByRole('menuitem', {name: 'Save Code…', exact: true}).click()
  const pending = page.waitForEvent('download')
  await page.getByRole('button', {name: 'Download C#', exact: true}).click()
  const source = await fs.readFile((await (await pending).path())!, 'utf8')
  expect(source).not.toMatch(/\bv_[0-9A-F]{4}\b|goto IL_|IL_[0-9A-F]{4}:/)
  expect(source).toContain('this.get_NotAProperty()')
  expect(source).toContain('catch (System.Exception')
  // Compile the saved repository-owned method with its fixture's declarations.
  // No uploaded or reconstructed assembly is ever run.
  const directory = testInfo.outputPath('reconstructed')
  await fs.mkdir(directory, {recursive: true})
  await fs.writeFile(path.join(directory, 'Readability.csproj'), '<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework><DebugType>none</DebugType><DebugSymbols>false</DebugSymbols></PropertyGroup></Project>')
  await fs.writeFile(path.join(directory, 'FormValues.cs'), `namespace ILens.Readability; public sealed class FormValues { public string Text { get; set; } = "fixture"; public bool Checked { get; set; } public string get_NotAProperty() => "method";\n${source}\n}`)
  const build = execFileSync('dotnet', ['build', directory, '--nologo', '-p:UseSharedCompilation=false'], {encoding: 'utf8', timeout: 60_000})
  expect(build).toContain('0 Error(s)')
  await page.getByRole('button', {name: 'Close export'}).click()
  await page.getByRole('tab', {name: 'IL', exact: true}).click()
  await expect(page.getByLabel('IL code viewer')).toContainText('stelem.ref')
  await expect(page.getByLabel('IL code viewer')).toContainText('get_Text')
  expect(errors).toEqual([])
})
