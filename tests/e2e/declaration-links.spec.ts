import {expect, test} from './test'
import path from 'node:path'

test('declaration types, attributes and member names navigate on a plain click', async ({page}) => {
  const errors: string[] = []
  page.on('pageerror', e => errors.push(e.message))
  await page.goto('./')
  await expect(page.getByLabel('Open assembly files')).toBeEnabled()
  await page.getByLabel('Open assembly files').setInputFiles(path.resolve('samples/fixtures/ILens.Patterns.dll'))
  await page.keyboard.press('ControlOrMeta+k')
  await page.getByLabel('Search members and strings').fill('Calculator')
  await page.locator('.search-results > button').filter({has: page.locator('strong', {hasText: /^Calculator$/})}).click()
  await page.getByRole('tab', {name: 'C#', exact: true}).click()
  const viewer = page.getByLabel('C# code viewer')
  await expect(viewer).toContainText('public class Calculator : ICalculator')
  // Base interface in the type header.
  await viewer.locator('.cursor-pointer').filter({hasText: /^ICalculator$/}).first().click()
  await expect(viewer).toContainText('public interface ICalculator')
  await page.getByLabel('Navigate back', {exact: true}).click()
  await expect(viewer).toContainText('public class Calculator : ICalculator')
  // A declared member name opens that member.
  await viewer.locator('.cursor-pointer').filter({hasText: /^Value$/}).first().click()
  await expect(viewer).toContainText('public int Value { get; set; }')
  await expect(viewer).not.toContainText('public class Calculator')
  // Selecting text by dragging over a link does not navigate.
  await page.getByLabel('Navigate back', {exact: true}).click()
  const link = viewer.locator('.cursor-pointer').filter({hasText: /^ICalculator$/}).first()
  const box = (await link.boundingBox())!
  await page.mouse.move(box.x + 1, box.y + box.height / 2)
  await page.mouse.down()
  await page.mouse.move(box.x + box.width - 1, box.y + box.height / 2, {steps: 4})
  await page.mouse.up()
  await expect(viewer).toContainText('public class Calculator : ICalculator')
  expect(errors).toEqual([])
})
