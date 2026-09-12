import {defineConfig} from '@playwright/test'

export default defineConfig({
  testDir: './tests/e2e',
  outputDir: 'test-results/dev',
  fullyParallel: false,
  workers: process.env.CI ? 2 : undefined,
  reporter: [['list'], ['html', {outputFolder: 'playwright-report/dev', open: 'never'}]],
  use: {baseURL: 'http://localhost:5173', viewport: {width: 1512, height: 982}, screenshot: 'only-on-failure', trace: 'retain-on-failure'},
  webServer: {command: 'node node_modules/rari/dist/cli.mjs dev', url: 'http://localhost:5173', reuseExistingServer: !process.env.CI, timeout: 120_000},
})
