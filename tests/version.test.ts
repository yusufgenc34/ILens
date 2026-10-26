import {it, expect} from 'vitest'
import {readFileSync} from 'node:fs'
import {version} from '../src/lib/version'

const read = (file: string) => readFileSync(new URL(`../${file}`, import.meta.url), 'utf8')

it('states one release version everywhere the build and the status bar read it', () => {
 expect(version).toMatch(/^\d+\.\d+\.\d+$/)
 expect(JSON.parse(read('package.json')).version).toBe(version)
 expect(JSON.parse(read('package-lock.json')).version).toBe(version)
 expect(JSON.parse(read('package-lock.json')).packages[''].version).toBe(version)
 expect(read('Cargo.toml')).toContain(`\n[workspace.package]\nversion = "${version}"\n`)
 expect(read('CHANGELOG.md')).toMatch(new RegExp(`^## ${version.replace(/\./g, '\\.')} `, 'm'))
})
