import {defineConfig} from '@playwright/test'

// Point the same browser tests at Docker without launching a second server.
const externalURL = process.env.PLAYWRIGHT_BASE_URL
export default defineConfig({
  testDir: './tests/e2e',
  testIgnore: '**/wasm.spec.ts',
  outputDir: 'test-results/production',
  fullyParallel: false,
  workers: process.env.CI ? 2 : undefined,
  reporter: [['list'], ['html', {outputFolder: 'playwright-report/production', open: 'never'}]],
  use: {baseURL: externalURL ?? 'http://localhost:3000', viewport: {width: 1512, height: 982}, screenshot: 'only-on-failure', trace: 'retain-on-failure'},
  webServer: externalURL ? undefined : {command: 'npm start', url: 'http://localhost:3000', reuseExistingServer: false, timeout: 60_000},
})
