import {expect, test} from './test'
import path from 'node:path'

const openPacked = async (page: import('@playwright/test').Page, fixture: string) => {
  await page.goto('./')
  await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(path.resolve(fixture))
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('1')
  await page.getByRole('menuitem', {name: 'Extras', exact: true}).click()
  await page.getByRole('menuitem', {name: 'Packed Assemblies…', exact: true}).click()
  return page.getByRole('dialog', {name: 'Packed assemblies', exact: true})
}

test('a payload carried in a resource is listed and opens into the workspace', async ({page}) => {
  const dialog = await openPacked(page, 'samples/fixtures/ILens.Packed.dll')
  await expect(dialog).toContainText('ILens.Packed.Payload.dll')
  await expect(dialog).toContainText('ILens.Example, Version=')
  // A resource that only starts like a PE is reported, never decoded or guessed at.
  await expect(dialog).toContainText('ILens.Packed.Scrambled.bin')
  await dialog.getByRole('button', {name: 'Open in workspace', exact: true}).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('2')
  // The payload is now a workspace assembly of its own, beside the host it came from.
  const explorer = page.getByRole('complementary', {name: 'Assembly Explorer'})
  await expect(explorer).toContainText('ILens.Packed')
  await expect(explorer).toContainText('ILens.Example')
  await expect(page.locator('.status-message')).toContainText('ILens.Packed.Payload.dll')
})

test('an assembly with no payload says so instead of listing nothing', async ({page}) => {
  const dialog = await openPacked(page, 'samples/fixtures/ILens.Example.dll')
  await expect(dialog).toContainText('No assembly is stored in this file')
  await expect(dialog.getByRole('button', {name: 'Open in workspace'})).toHaveCount(0)
})
