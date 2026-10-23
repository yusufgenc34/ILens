import {devices, type Project} from '@playwright/test'
// Desktop suites run in all three engines; mobile layouts run in phone emulation only.
// Select engines with --project, e.g. `npm run test:e2e -- --project=firefox`.
const desktop = {width: 1512, height: 982}
export function browserProjects(ignore: string[] = []): Project[] {
  const desktopIgnore = [...ignore, '**/*mobile*.spec.ts']
  return [
    {name: 'chromium', testIgnore: desktopIgnore, use: {...devices['Desktop Chrome'], viewport: desktop}},
    {name: 'firefox', testIgnore: desktopIgnore, use: {...devices['Desktop Firefox'], viewport: desktop}},
    {name: 'webkit', testIgnore: desktopIgnore, use: {...devices['Desktop Safari'], viewport: desktop}},
    {name: 'mobile-chrome', testMatch: '**/*mobile*.spec.ts', use: {...devices['Pixel 7']}},
    {name: 'mobile-safari', testMatch: '**/*mobile*.spec.ts', use: {...devices['iPhone 14']}},
  ]
}
