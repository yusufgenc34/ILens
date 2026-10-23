// Regenerates crates/decompiler-core/data/bcl.txt from the installed .NET SDK's
// Microsoft.NETCore.App.Ref reference assemblies (metadata only; nothing is executed).
import {spawnSync} from 'node:child_process'
import {readdirSync, writeFileSync} from 'node:fs'
import path from 'node:path'
const sdks = spawnSync('dotnet', ['--list-sdks'], {encoding: 'utf8'})
if (sdks.status !== 0) throw new Error('The .NET SDK is required to regenerate the BCL summary.')
const root = path.dirname(sdks.stdout.trim().split('\n').at(-1).match(/\[(.*)\]/)[1])
const pack = path.join(root, 'packs', 'Microsoft.NETCore.App.Ref')
const version = readdirSync(pack).sort((a, b) => a.localeCompare(b, 'en', {numeric: true})).at(-1)
const framework = readdirSync(path.join(pack, version, 'ref')).sort().at(-1)
const directory = path.join(pack, version, 'ref', framework)
const result = spawnSync('cargo', ['run', '-q', '--locked', '-p', 'decompiler-core', '--example', 'bcl_summary', '--', directory], {encoding: 'utf8', maxBuffer: 64 * 1024 * 1024})
if (result.status !== 0) throw new Error(result.stderr)
writeFileSync('crates/decompiler-core/data/bcl.txt', result.stdout)
console.log(`BCL summary from Microsoft.NETCore.App.Ref ${version} (${framework}): ${result.stdout.split('\n').filter(l => l.startsWith('T ')).length} types`)
