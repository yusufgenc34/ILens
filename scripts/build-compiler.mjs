// Builds the optional C# editing component: Roslyn on .NET WebAssembly. It is
// downloaded by the browser only when a method is edited in C#, and it compiles
// source against assemblies as metadata; it never runs them. Requires the .NET 10 SDK.
import {spawnSync} from 'node:child_process'
import {cpSync, readdirSync, rmSync} from 'node:fs'
import path from 'node:path'

const root = path.resolve(import.meta.dirname, '..')
// Stale files from earlier publishes would otherwise be copied too.
rmSync(path.join(root, 'compiler/bin/Release/net10.0/publish'), {recursive: true, force: true})
const publish = spawnSync('dotnet', ['publish', 'compiler', '-c', 'Release'], {cwd: root, stdio: 'inherit'})
if (publish.status !== 0) process.exit(publish.status ?? 1)
const source = path.join(root, 'compiler/bin/Release/net10.0/publish/wwwroot/_framework')
const target = path.join(root, 'public/compiler/_framework')
rmSync(target, {recursive: true, force: true})
// Precompressed copies are left to the server; the files themselves are enough.
cpSync(source, target, {recursive: true, filter: file => !/\.(br|gz)$/.test(file)})
const files = readdirSync(target)
console.log(`C# compiler component: ${files.length} files in public/compiler/_framework`)
