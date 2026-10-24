import {expect, test} from './test'

test('first visit opens the example and walks through the workspace tour', async ({page}, testInfo) => {
  // Runs after the shared fixture marks the tour as seen: this is a first visit.
  await page.addInitScript(() => localStorage.removeItem('ilens:tour-seen'))
  await page.goto('./')
  const tour = page.getByRole('dialog', {name: 'Welcome to ILens'})
  await expect(tour).toBeVisible()
  await expect(page.getByRole('main')).toHaveAttribute('data-assembly-id', '1')
  await expect(page.getByLabel('C# code viewer')).toContainText('public class Greeter')
  for (const title of ['Assembly Explorer', 'Views', 'Code', 'Inspector', 'Toolbar', 'Menus', 'Ready']) {
    await page.getByRole('button', {name: 'Next'}).click()
    await expect(page.getByRole('dialog', {name: title})).toBeVisible()
  }
  await page.screenshot({path: testInfo.outputPath('tour.png'), animations: 'disabled'})
  await page.getByRole('button', {name: 'Finish'}).click()
  await expect(page.getByRole('dialog', {name: 'Ready'})).toHaveCount(0)
  // Not again on the next visit; Help restarts it.
  expect(await page.evaluate(() => localStorage.getItem('ilens:tour-seen'))).toBe('1')
  await page.getByRole('menuitem', {name: 'Help', exact: true}).click()
  await page.getByRole('menuitem', {name: 'Take the Tour', exact: true}).click()
  await expect(page.getByRole('dialog', {name: 'Welcome to ILens'})).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog', {name: 'Welcome to ILens'})).toHaveCount(0)
})
test('the example opens on its first class and reads like source', async ({page}) => {
  await page.goto('./')
  await page.getByRole('button', {name: 'Open the example'}).click()
  const code = page.getByLabel('C# code viewer')
  await expect(code).toContainText('public class Greeter')
  await page.getByRole('treeitem', {name: /^Calculator/}).click()
  await expect(code).toContainText('if (values.Length == 0)')
  await expect(code).not.toContainText('nuint')
})
