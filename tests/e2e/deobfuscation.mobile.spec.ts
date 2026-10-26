import {test,expect} from './test'
import path from 'node:path'
test('deobfuscation dialog remains usable on a phone',async({page},info)=>{
  await page.goto('./');await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(path.resolve('samples/fixtures/ILens.Deobfuscation.dll'))
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('1')
  await page.getByRole('menuitem',{name:'Extras',exact:true}).click();await page.getByRole('menuitem',{name:'Deobfuscation Options…'}).click()
  const dialog=page.getByRole('dialog',{name:'Deobfuscate assembly',exact:true})
  await dialog.getByRole('checkbox',{name:/Evaluate pure string/}).check()
  await dialog.getByRole('button',{name:'Run deobfuscation'}).click()
  await expect(dialog.getByRole('region',{name:'Deobfuscation report'})).toContainText('2 strings',{timeout:20000})
  await expect(dialog.getByRole('heading',{name:'Report · partial'})).toBeInViewport()
  await expect(dialog.getByRole('button',{name:'Done',exact:true})).toBeInViewport({ratio:1})
  const bounds=await dialog.boundingBox();expect(bounds!.width).toBeLessThanOrEqual(page.viewportSize()!.width)
  await page.screenshot({path:info.outputPath('deobfuscation-mobile.png'),animations:'disabled'})
  await dialog.getByRole('button',{name:'Done',exact:true}).click();await expect(dialog).toHaveCount(0)
})
