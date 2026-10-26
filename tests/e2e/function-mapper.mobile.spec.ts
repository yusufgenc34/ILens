import {expect, test} from './test'
import path from 'node:path'

test('the relationship map stays usable on a phone', async ({page}, info) => {
  await page.goto('./')
  await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(path.resolve('samples/fixtures/ILens.Patterns.dll'))
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('1')
  await page.getByRole('menuitem', {name: 'Extras', exact: true}).click()
  await page.getByRole('menuitem', {name: 'Function Mapper…', exact: true}).click()
  const mapper = page.getByRole('dialog', {name: 'Relationship Mapper', exact: true})
  const graph = mapper.getByRole('img', {name: 'Assembly relationship map'})
  await expect(graph).toBeVisible()
  // The dialog fits the viewport, and the controls over the map are reachable.
  const bounds = await mapper.boundingBox()
  expect(bounds!.width).toBeLessThanOrEqual(page.viewportSize()!.width)
  await expect(mapper.getByLabel('Search the map')).toBeInViewport()
  await expect(mapper.getByRole('button', {name: 'Close relationship mapper'})).toBeInViewport({ratio: 1})
  const calculator = graph.getByRole('button', {name: 'Calculator (class)', exact: true})
  await calculator.tap()
  await expect(mapper.getByText('Focused on')).toBeVisible()
  await expect(mapper.getByRole('button', {name: 'Show whole map', exact: true})).toBeInViewport()
  await page.screenshot({path: info.outputPath('relationship-map-mobile.png'), animations: 'disabled'})
  await mapper.getByRole('button', {name: 'Close relationship mapper'}).click()
  await expect(mapper).toHaveCount(0)
})
