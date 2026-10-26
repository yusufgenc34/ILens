import {defineConfig} from '@playwright/test'
import {browserProjects} from './tests/e2e/projects'

export default defineConfig({
  testDir: './tests/e2e',
  outputDir: 'test-results/dev',
  fullyParallel: false,
  // A loaded machine can take longer than the 5s default to finish a WASM load.
  expect: {timeout: 10_000},
  workers: process.env.CI ? 2 : undefined,
  reporter: [['list'], ['html', {outputFolder: 'playwright-report/dev', open: 'never'}]],
  use: {baseURL: 'http://localhost:5173', screenshot: 'only-on-failure', trace: 'retain-on-failure'},
  projects: browserProjects(),
  webServer: {command: 'node node_modules/rari/dist/cli.mjs dev', url: 'http://localhost:5173', reuseExistingServer: !process.env.CI, timeout: 120_000},
})
