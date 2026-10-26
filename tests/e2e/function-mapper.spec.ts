import {expect, test} from './test'
import path from 'node:path'

const open = async (page: import('@playwright/test').Page, fixture: string) => {
  await page.goto('./')
  await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(path.resolve(fixture))
  await expect(page.getByLabel('Loaded assemblies')).toHaveText('1')
  await page.getByRole('menuitem', {name: 'Extras', exact: true}).click()
  await page.getByRole('menuitem', {name: 'Function Mapper…', exact: true}).click()
  return page.getByRole('dialog', {name: 'Relationship Mapper', exact: true})
}

test('the mapper is only a map and shows every type, including unconnected ones', async ({page}) => {
  const errors: string[] = []
  page.on('pageerror', e => errors.push(e.message))
  // Every type in this fixture is standalone; the map must still show all of them.
  const mapper = await open(page, 'samples/fixtures/ILens.Example.dll')
  const graph = mapper.getByRole('img', {name: 'Assembly relationship map'})
  await expect(graph).toBeVisible()
  await expect(mapper).not.toContainText('No types to diagram')
  for (const name of ['Calculator (class)', 'Person (class)', 'Greeter (class)', 'Shape (enum)', 'Geometry (class)']) {
    await expect(graph.getByRole('button', {name, exact: true})).toBeVisible()
  }
  // Nothing else lives in this dialog: no list, no search, no export.
  await expect(mapper.getByRole('radio')).toHaveCount(0)
  await expect(mapper.getByRole('listbox')).toHaveCount(0)
  await expect(mapper.getByRole('button', {name: 'CSV', exact: true})).toHaveCount(0)
  expect(errors).toEqual([])
})

test('clicking a type focuses it and shows its own functions', async ({page}) => {
  const mapper = await open(page, 'samples/fixtures/ILens.Patterns.dll')
  const graph = mapper.getByRole('img', {name: 'Assembly relationship map'})
  const calculator = graph.getByRole('button', {name: 'Calculator (class)', exact: true})
  await expect(calculator).toBeVisible()
  await expect(graph.getByRole('button', {name: 'function Add', exact: true})).toHaveCount(0)
  // Unrelated types are on the full map but drop away once a node is focused.
  await expect(graph.getByRole('button', {name: 'Mode (enum)', exact: true})).toBeVisible()
  await calculator.click()
  await expect(mapper.getByText('Focused on')).toBeVisible()
  await expect(graph.getByRole('button', {name: 'function Add', exact: true})).toBeVisible()
  await expect(graph.getByRole('button', {name: 'Mode (enum)', exact: true})).toHaveCount(0)
  await mapper.getByRole('button', {name: 'Show whole map', exact: true}).click()
  await expect(graph.getByRole('button', {name: 'Mode (enum)', exact: true})).toBeVisible()
  await calculator.dblclick()
  await expect(mapper).toHaveCount(0)
  await expect(page.getByLabel('C# code viewer')).toContainText('class Calculator')
})

test('searching the map filters by name and by string literal', async ({page}) => {
  const mapper = await open(page, 'samples/fixtures/ILens.Deobfuscation.dll')
  const graph = mapper.getByRole('img', {name: 'Assembly relationship map'})
  const search = mapper.getByLabel('Search the map')
  // This literal is loaded only by Patterns.ExternalCall.
  await search.fill('fixture-do-not-open')
  await expect(mapper.getByText(/\d+ match/)).toBeVisible()
  await expect(graph.getByRole('button', {name: 'function ExternalCall', exact: true})).toBeVisible()
  await search.fill('WithInitializer')
  await expect(graph.getByRole('button', {name: 'WithInitializer (class)', exact: true})).toBeVisible()
  await expect(graph.getByRole('button', {name: 'function ExternalCall', exact: true})).toHaveCount(0)
  await search.fill('zzz-no-such-thing')
  await expect(mapper.getByText(/Nothing matches/)).toBeVisible()
})

test('searching for unused shows the functions nothing in the assembly calls', async ({page}) => {
  const mapper = await open(page, 'samples/fixtures/ILens.Patterns.dll')
  const graph = mapper.getByRole('img', {name: 'Assembly relationship map'})
  await mapper.getByLabel('Search the map').fill('unused')
  await expect(mapper.getByText(/\d+ match/)).toBeVisible()
  // Public API with no caller inside this assembly is exactly what this finds; a
  // static scan cannot see reflection, so nothing is removed on the strength of it.
  await expect(graph.getByRole('button', {name: /^function /}).first()).toBeVisible()
  // A type name is not a tag, so it does not answer this search.
  await expect(graph.getByRole('button', {name: 'Mode (enum)', exact: true})).toHaveCount(0)
})

test('a user string lists the methods that load it', async ({page}) => {
  await page.goto('./')
  await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(path.resolve('samples/fixtures/ILens.Deobfuscation.dll'))
  await page.keyboard.press('ControlOrMeta+k')
  await page.getByLabel('Search members and strings').fill('fixture-do-not-open')
  await page.locator('.search-results > button').first().click()
  const viewer = page.getByLabel('C# code viewer')
  await expect(viewer).toContainText('Used by 1 method')
  await expect(viewer).toContainText('ExternalCall')
})
