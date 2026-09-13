import {defineConfig} from '@playwright/test'

const externalURL = process.env.PLAYWRIGHT_PAGES_URL
const basePath = process.env.ILENS_PAGES_BASE ?? '/ILens/'
const localURL = `http://127.0.0.1:4173${basePath}`
export default defineConfig({
  testDir: './tests',
  testMatch: ['e2e/*.spec.ts', 'pages/*.spec.ts'],
  testIgnore: '**/wasm.spec.ts',
  outputDir: 'test-results/pages',
  fullyParallel: false,
  workers: 2,
  reporter: [['list'], ['html', {outputFolder: 'playwright-report/pages', open: 'never'}]],
  use: {baseURL: externalURL ?? localURL, viewport: {width: 1512, height: 982}, screenshot: 'only-on-failure', trace: 'retain-on-failure'},
  webServer: externalURL ? undefined : {command: 'npm run preview:pages', url: localURL, reuseExistingServer: false, timeout: 60_000},
})
