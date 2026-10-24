import {defineConfig} from '@playwright/test'
import base from './playwright.config'
import {browserProjects} from './tests/e2e/projects'

// Firefox in the Linux Playwright container, against the dev server on the host.
// `npm run test:e2e:firefox` forwards the host's ports to the container's localhost.
export default defineConfig({
  ...base,
  outputDir: 'test-results/docker',
  reporter: [['list']],
  use: {...base.use, baseURL: 'http://localhost:5173'},
  webServer: undefined,
  projects: browserProjects().filter(project => project.name === 'firefox'),
})
