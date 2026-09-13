import {test, expect} from '@playwright/test'

test('static host serves worker, WASM, styles and sample under the site base without server endpoints', async ({page, context, baseURL}, testInfo) => {
  const origin = new URL(baseURL!).origin
  const prefix = new URL(baseURL!).pathname
  const requests: {url: string; method: string}[] = []
  const assets: {url: string; status: number; type: string}[] = []
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  page.on('console', message => {if (message.type() === 'error') errors.push(message.text())})
  context.on('request', request => requests.push({url: request.url(), method: request.method()}))
  context.on('response', response => {
    if (/\.(css|js|wasm|dll)(?:\?|$)/.test(response.url())) assets.push({url: response.url(), status: response.status(), type: response.headers()['content-type'] ?? ''})
  })
  await page.goto('./')
  await expect(page).toHaveTitle('ILens — .NET decompiler')
  await expect(page.getByRole('button', {name: 'Explore a compiled sample'})).toBeEnabled()
  await page.getByRole('button', {name: 'Explore a compiled sample'}).click()
  await expect(page.getByLabel('Primary assembly')).toHaveValue('1')
  await page.keyboard.press('ControlOrMeta+k')
  await page.getByLabel('Search members and strings').fill('Calculator.Sum')
  await page.getByRole('button', {name: /Sum.*Calculator.Sum/}).click()
  await expect(page.getByLabel('C# code viewer')).toContainText('for (')
  await expect(page.locator('.workspace')).toHaveCSS('display', 'grid')
  await expect(page.locator('.cm-scroller')).toHaveCSS('font-family', /monospace/)
  await page.screenshot({path: testInfo.outputPath('pages-workspace.png'), fullPage: true})
  expect(assets.some(asset => asset.url.endsWith('.wasm') && asset.type.includes('application/wasm'))).toBe(true)
  expect(assets.some(asset => asset.url.endsWith('samples/ILens.Patterns.dll'))).toBe(true)
  expect(assets.some(asset => asset.url.endsWith('.css') && asset.type.includes('text/css'))).toBe(true)
  expect(assets.every(asset => asset.status === 200 || asset.status === 304)).toBe(true)
  expect(requests.every(request => request.method === 'GET')).toBe(true)
  expect(requests.every(request => new URL(request.url).origin === origin && new URL(request.url).pathname.startsWith(prefix))).toBe(true)
  expect(requests.some(request => /\/(?:_rari|_rsc|api)(?:\/|\?)/.test(request.url))).toBe(false)
  expect(errors).toEqual([])
  await page.reload()
  await expect(page.getByRole('heading', {name: /Look inside/})).toBeVisible()
})
