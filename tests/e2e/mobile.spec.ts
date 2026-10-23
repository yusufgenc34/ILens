import {expect, test, type Page} from './test'
import path from 'node:path'
// Runs only in the phone projects (Pixel 7 and iPhone 14 emulation).
const noHorizontalOverflow = async (page: Page) => expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(0)
test('phone layout uses drawers, fits the screen and supports touch menus', async ({page}, info) => {
  await page.goto('./'); await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await noHorizontalOverflow(page)
  await page.getByLabel('Open assembly files').setInputFiles(path.resolve('samples/fixtures/ILens.Workspace.dll'))
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id', '1')
  await noHorizontalOverflow(page)
  const explorer = page.getByRole('complementary', {name: 'Assembly Explorer'})
  await expect(explorer).toBeHidden()
  const code = await page.getByRole('main').boundingBox()
  expect(code!.width).toBeGreaterThan(page.viewportSize()!.width - 2)

  await page.getByRole('button', {name: 'Show assembly explorer'}).tap()
  await expect(explorer).toBeVisible()
  const row = page.getByRole('treeitem', {name: 'ILens.Samples', exact: true})
  expect((await row.boundingBox())!.height).toBeGreaterThanOrEqual(36)
  await row.tap()
  await page.getByRole('treeitem', {name: 'WorkspacePatterns', exact: true}).locator('.disclosure').tap()
  const method = page.locator('[role=treeitem][data-kind=method]').first()
  // Long press opens the declaration menu without closing the drawer.
  const box = (await method.boundingBox())!
  await method.dispatchEvent('pointerdown', {pointerType: 'touch', clientX: box.x + 40, clientY: box.y + box.height / 2, isPrimary: true, bubbles: true})
  const menu = page.getByRole('menu', {name: /actions$/})
  await expect(menu).toBeVisible()
  await method.dispatchEvent('pointerup', {pointerType: 'touch', isPrimary: true, bubbles: true})
  await expect(explorer).toBeVisible()
  const menuBox = (await menu.boundingBox())!
  expect(menuBox.x + menuBox.width).toBeLessThanOrEqual(page.viewportSize()!.width)
  await page.keyboard.press('Escape')

  await method.tap()
  await expect(explorer).toBeHidden()
  await expect(page.getByLabel('C# code viewer')).toBeVisible()
  await page.screenshot({path: info.outputPath('mobile-code.png')})

  await page.getByRole('button', {name: 'Show inspector'}).tap()
  await expect(page.getByTestId('protection-status')).toHaveText('No findings')
  await page.locator('.drawer-backdrop').tap({position: {x: 10, y: 200}})
  await expect(page.getByTestId('protection-status')).toBeHidden()

  await page.getByRole('menuitem', {name: 'Help', exact: true}).tap()
  const help = page.getByRole('menu', {name: 'Help', exact: true})
  const helpBox = (await help.boundingBox())!
  expect(helpBox.x).toBeGreaterThanOrEqual(0); expect(helpBox.x + helpBox.width).toBeLessThanOrEqual(page.viewportSize()!.width)
  await noHorizontalOverflow(page)
})
