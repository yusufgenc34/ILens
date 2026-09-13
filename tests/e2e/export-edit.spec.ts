import {test, expect, type Page} from '@playwright/test'
import fs from 'node:fs/promises'
import path from 'node:path'
import {execFileSync} from 'node:child_process'
const dependency = path.resolve('samples/fixtures/ILens.Dependency.dll')
const patterns = path.resolve('samples/fixtures/ILens.Patterns.dll')
async function open(page: Page, file: string) {await page.goto('./'); await expect(page.getByLabel('Open assembly files')).toBeEnabled(); await page.getByLabel('Open assembly files').setInputFiles(file); await expect(page.getByLabel('Primary assembly')).toHaveValue('1')}
async function select(page: Page, query: string, name: RegExp) {await page.getByRole('menuitem', {name: 'Edit', exact: true}).click(); await page.getByRole('menuitem', {name: 'Search Assembly…', exact: true}).click(); await page.getByLabel('Search members and strings').fill(query); await page.getByRole('button', {name}).click(); await expect(page.getByLabel('C# code viewer')).toContainText('return (value *')}
function archiveData(buffer: Buffer): {names: string[]; files: Record<string, string>; report: {outcome: string; members: unknown[]; resources: {path: string | null}[]; projects?: {project: string; report: string}[]}} {
  return JSON.parse(execFileSync('python3', ['-c', 'import sys,io,zipfile,json; z=zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read())); assert z.testzip() is None; assert all(i.date_time==(1980,1,1,0,0,0) and not i.extra for i in z.infolist()); print(json.dumps({"names":z.namelist(),"files":{n:z.read(n).decode() for n in z.namelist() if n.endswith((".cs",".csproj",".il",".sln",".json"))},"report":json.loads(z.read("export-report.json" if "export-report.json" in z.namelist() else "_ilens/export-report.json"))}))'], {input: buffer, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024}))
}
test('downloads exact method output and a fixture project that compiles without executing it', async ({page}, testInfo) => {
  test.setTimeout(120_000)
  await open(page, dependency); await select(page, 'Arithmetic.Twice', /Twice.*Arithmetic.Twice/)
  await page.getByRole('menuitem', {name: 'File', exact: true}).click(); await page.getByRole('menuitem', {name: 'Save Code…', exact: true}).click()
  let download = page.waitForEvent('download'); await page.getByRole('button', {name: 'Download C#', exact: true}).click()
  const source = await download; expect(source.suggestedFilename()).toMatch(/Twice_06000001.cs/); expect(await fs.readFile((await source.path())!, 'utf8')).toContain('return (value * 2)')
  download = page.waitForEvent('download'); await page.getByRole('button', {name: 'Download IL', exact: true}).click(); expect(await fs.readFile((await (await download).path())!, 'utf8')).toContain('ldc.i4.2')
  await page.getByRole('button', {name: 'Close export'}).click()
  await page.getByRole('menuitem', {name: 'File', exact: true}).click(); await page.getByRole('menuitem', {name: 'Export to Project…', exact: true}).click()
  download = page.waitForEvent('download'); await page.getByRole('button', {name: 'Export', exact: true}).click()
  const result = await download; const data = await fs.readFile((await result.path())!); const zip = archiveData(data)
  expect(zip.names).toContain('ILens.Dependency.sln'); expect(zip.files['ILens.Dependency/ILens.Dependency.csproj']).toContain('<TargetFramework>net10.0'); expect(zip.files['ILens.Dependency/ILens.Dependency.csproj']).toContain('<AssemblyName>ILens.Dependency</AssemblyName>')
  expect(zip.names).toContain('ILens.Dependency/Arithmetic.cs'); expect(zip.names).toContain('ILens.Dependency/Properties/AssemblyInfo.cs'); expect(zip.names.some(n => /_[0-9a-f]{8}\.cs$/.test(n))).toBe(false)
  const directory = testInfo.outputPath('reconstructed'); await fs.mkdir(directory, {recursive: true})
  execFileSync('python3', ['-c', 'import sys,io,zipfile; zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read())).extractall(sys.argv[1])', directory], {input: data})
  // This is only the repository-owned arithmetic fixture. Build it, never run it.
  const build = execFileSync('dotnet', ['build', path.join(directory, 'ILens.Dependency.sln'), '--nologo', '-p:UseSharedCompilation=false', '-p:DebugType=none', '-p:DebugSymbols=false'], {encoding: 'utf8', timeout: 60_000})
  expect(build).toContain('0 Error(s)')
  expect(execFileSync('dotnet', ['sln', path.join(directory, 'ILens.Dependency.sln'), 'list'], {encoding: 'utf8'})).toContain('ILens.Dependency.csproj')
  await expect(page.getByRole('button', {name: 'Close', exact: true})).toBeInViewport({ratio: 1})
  const dialogBounds = await page.getByRole('dialog', {name: 'Export to Project', exact: true}).boundingBox()
  expect(Math.abs(dialogBounds!.x + dialogBounds!.width / 2 - 756)).toBeLessThan(2)
  await page.screenshot({path: testInfo.outputPath('export-dialog.png')})
  await page.setViewportSize({width: 1024, height: 600})
  await expect(page.getByRole('button', {name: 'Close', exact: true})).toBeInViewport({ratio: 1})
  await expect(page.getByRole('button', {name: 'Close project export'})).toBeInViewport({ratio: 1})
  await page.setViewportSize({width: 1512, height: 982})
  await page.getByRole('button', {name: 'Close project export'}).click()
  await page.getByRole('treeitem', {name: /^Arithmetic/}).click({button: 'right'})
  download = page.waitForEvent('download'); await page.getByRole('button', {name: 'Download type C#', exact: true}).click()
  const typeSource = await download
  expect(typeSource.suggestedFilename()).toBe('Arithmetic_02000002.cs')
  expect(await fs.readFile((await typeSource.path())!, 'utf8')).toContain('public static class Arithmetic')

})
test('source archive reports partial reconstruction, includes resources, and can be cancelled', async ({page}) => {
  test.setTimeout(60_000)
  // Keep a real WASM export step in flight long enough to test cancellation
  // deterministically; only message timing changes, never engine results.
  await page.addInitScript(() => {
    const post = Worker.prototype.postMessage
    Worker.prototype.postMessage = function (this: Worker, message: unknown, options?: Transferable[] | StructuredSerializeOptions) {
      const send = () => post.call(this, message, Array.isArray(options) ? {transfer: options} : options)
      if (message && typeof message === 'object' && 'op' in message && message.op === 'stepExport') setTimeout(send, 60)
      else send()
    }
  })
  await open(page, patterns); await page.getByLabel('Add dependency files').setInputFiles(dependency); await expect(page.getByText('1 dependency assemblies loaded', {exact: true})).toBeVisible()
  await page.getByRole('menuitem', {name: 'File', exact: true}).click(); await page.getByRole('menuitem', {name: 'Export to Project…', exact: true}).click(); await page.getByRole('checkbox', {name: /ILens.Dependency/}).check()
  const downloads: string[] = []; page.on('download', d => downloads.push(d.suggestedFilename()))
  await page.getByRole('button', {name: 'Export', exact: true}).click(); await page.getByRole('button', {name: 'Cancel export', exact: true}).click()
  await expect(page.getByRole('dialog', {name: 'Export to Project', exact: true})).toContainText('Export cancelled. No file was downloaded.'); expect(downloads).toEqual([])
  const download = page.waitForEvent('download'); await page.getByRole('button', {name: 'Export', exact: true}).click()
  const data = archiveData(await fs.readFile((await (await download).path())!))
  expect(data.report.outcome).toBe('partial'); expect(data.report.projects).toHaveLength(2)
  const report = JSON.parse(data.files['ILens.Patterns/_ilens/export-report.json'])
  expect(report.members.length).toBeGreaterThan(60); expect(report.resources.some((r: {path: string | null}) => r.path)).toBe(true)
  expect(data.files['ILens.Patterns/ILens.Patterns.csproj']).toContain('<ProjectReference Include="../ILens.Dependency/ILens.Dependency.csproj"')
  expect(data.files['ILens.Patterns.sln']).toContain('ILens.Dependency\\ILens.Dependency.csproj')
  expect(data.names.some(n => n.includes('/_ilens/il/') && n.endsWith('.il'))).toBe(true)
  expect(data.files[Object.keys(data.files).find(n => n.endsWith('/Calculator.cs'))!]).toContain('public int Value')
})
test('edits IL with undo, rejects invalid types, validates and reopens a real modified DLL', async ({page}, testInfo) => {
  await open(page, dependency); await select(page, 'Arithmetic.Twice', /Twice.*Arithmetic.Twice/)
  await expect(page.getByRole('button', {name: 'Edit IL', exact: true})).toHaveCount(0)
  await page.getByRole('menuitem', {name: 'Edit', exact: true}).click()
  await expect(page.getByRole('menuitem', {name: 'Edit IL…', exact: true})).toHaveCount(0)
  await page.keyboard.press('Escape')
  for (const view of ['IL', 'C#', 'Metadata', 'Hex', 'IL']) {
    await page.getByRole('tab', {name: view, exact: true}).click()
    await expect(page.getByRole('button', {name: 'Edit IL', exact: true})).toHaveCount(view === 'IL' ? 1 : 0)
  }
  await page.locator('.code-toolbar').getByRole('button', {name: 'Edit IL', exact: true}).click()
  const opcode = page.getByLabel('Opcode IL_0001', {exact: true}); await expect(opcode).toHaveValue('ldc.i4.2')
  await opcode.fill('ldc.i4.3'); await page.getByLabel('Undo IL change').click(); await expect(opcode).toHaveValue('ldc.i4.2'); await page.getByLabel('Redo IL change').click(); await expect(opcode).toHaveValue('ldc.i4.3')
  await opcode.fill('ldnull'); await page.getByRole('button', {name: 'Validate and apply'}).click(); await expect(page.getByRole('alert')).toContainText('matching numeric stack types')
  await opcode.fill('ldc.i4.3'); await page.getByLabel('Insert before IL_0000').click(); await page.getByRole('button', {name: 'Validate and apply'}).click(); await expect(page.getByRole('dialog')).toContainText('Validation passed.')
  await page.screenshot({path: testInfo.outputPath('il-editor.png')})
  await page.getByRole('button', {name: 'Export modified assembly', exact: true}).click()
  const download = page.waitForEvent('download'); await page.getByRole('button', {name: 'Download modified assembly', exact: true}).click(); const output = await download
  expect(output.suggestedFilename()).toBe('ILens_Dependency.modified.dll'); const saved = testInfo.outputPath(output.suggestedFilename()); await output.saveAs(saved)
  const runtimes = execFileSync('dotnet', ['--list-runtimes'], {encoding: 'utf8'})
  const runtime = runtimes.split('\n').find(line => line.startsWith('Microsoft.NETCore.App 10.'))?.match(/^Microsoft.NETCore.App (\S+) \[(.+)\]$/)
  expect(runtime).toBeTruthy()
  const verification = execFileSync('dotnet', ['tool', 'run', 'ilverify', saved, '-r', path.join(runtime![2], runtime![1], '*.dll'), '-s', 'System.Private.CoreLib'], {encoding: 'utf8', timeout: 30_000})
  expect(verification).toContain('Verified')
  await page.getByRole('button', {name: 'Close export'}).click(); await page.getByRole('button', {name: 'Close workspace', exact: true}).click()
  await page.getByLabel('Open assembly files').setInputFiles(saved); await expect(page.getByLabel('Primary assembly')).toHaveValue('1'); await select(page, 'Arithmetic.Twice', /Twice.*Arithmetic.Twice/)
  await expect(page.getByLabel('C# code viewer')).toContainText('return (value * 3)')
})
