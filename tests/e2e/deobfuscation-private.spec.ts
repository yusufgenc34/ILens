// Optional local compatibility check. Never copy a private input into the repository.
// ILENS_LOCAL_PROTECTED_ASSEMBLY=/absolute/path/file.exe npm run test:production -- --project=chromium deobfuscation-private
import {test,expect} from './test'
import {readFile} from 'node:fs/promises'

test('local legacy resource profile runs in the real worker and opens an independent copy',async({page},info)=>{
  const input=process.env.ILENS_LOCAL_PROTECTED_ASSEMBLY
  test.skip(!input,'An explicitly supplied local compatibility sample is required')
  test.setTimeout(120_000)
  const errors:string[]=[],badResponses:string[]=[],uploads:string[]=[]
  page.on('pageerror',e=>errors.push(e.message))
  page.on('response',r=>{if(r.status()>=400)badResponses.push(`${r.status()} ${r.url()}`)})
  page.on('request',r=>{if(r.method()!=='GET')uploads.push(r.url())})
  await page.goto('./')
  await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(input!)
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('1',{timeout:20000})
  await page.getByRole('menuitem',{name:'Extras',exact:true}).click()
  await page.getByRole('menuitem',{name:'Deobfuscation Options…',exact:true}).click()
  const dialog=page.getByRole('dialog',{name:'Deobfuscate assembly',exact:true})
  await expect(dialog.getByRole('checkbox',{name:/Decode protected resource strings/})).toBeChecked()
  await expect(dialog.getByRole('button',{name:'Run deobfuscation'})).toBeEnabled()
  await dialog.getByRole('button',{name:'Run deobfuscation'}).click()
  await expect(dialog.getByRole('region',{name:'Deobfuscation report'})).toContainText('Recognized legacy Eazfuscator-compatible',{timeout:90000})
  const reportEvent=page.waitForEvent('download')
  await dialog.getByRole('button',{name:'Download report',exact:true}).click()
  const report=JSON.parse(await readFile((await (await reportEvent).path())!,'utf8'))
  expect(report.counts.strings).toBeGreaterThan(0)
  expect(report.changed_methods).toBeGreaterThan(0)
  expect(report.unresolved_calls).toBe(0)
  expect(report.notices.join(' ')).toContain('Output reparsed successfully')
  expect(await dialog.evaluate(e=>getComputedStyle(e).backgroundColor)).not.toBe('rgba(0, 0, 0, 0)')
  await expect(dialog.getByRole('button',{name:'Done',exact:true})).toBeInViewport({ratio:1})
  // The report contains counts/technical reasons, not decrypted application strings.
  await page.screenshot({path:info.outputPath('resource-string-report.png'),animations:'disabled'})
  await dialog.getByRole('button',{name:'Open copy in workspace'}).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('2')
  expect(errors).toEqual([]);expect(badResponses).toEqual([]);expect(uploads).toEqual([])
})
