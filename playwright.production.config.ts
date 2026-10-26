import {defineConfig} from '@playwright/test'
import {browserProjects} from './tests/e2e/projects'

// Point the same browser tests at Docker without launching a second server.
const externalURL = process.env.PLAYWRIGHT_BASE_URL
export default defineConfig({
  testDir: './tests/e2e',
  testIgnore: '**/wasm.spec.ts',
  outputDir: 'test-results/production',
  fullyParallel: false,
  // A loaded machine can take longer than the 5s default to finish a WASM load.
  expect: {timeout: 10_000},
  // This config also drives the container run, where WebKit needs several times
  // longer for the same work than it does against a local server.
  timeout: 60_000,
  workers: process.env.CI ? 2 : undefined,
  reporter: [['list'], ['html', {outputFolder: 'playwright-report/production', open: 'never'}]],
  use: {baseURL: externalURL ?? 'http://localhost:3000', screenshot: 'only-on-failure', trace: 'retain-on-failure'},
  projects: browserProjects(['**/wasm.spec.ts']),
  webServer: externalURL ? undefined : {command: 'npm start', url: 'http://localhost:3000', reuseExistingServer: false, timeout: 60_000},
})
