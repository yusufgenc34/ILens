import {spawnSync} from 'node:child_process'
import {copyFileSync, mkdirSync, readdirSync} from 'node:fs'
import path from 'node:path'
const result = spawnSync('dotnet', ['build', 'samples/Patterns', '-c', 'Release', '-o', 'samples/bin'], {stdio: 'inherit'})
if (result.status !== 0) process.exit(result.status ?? 1)
mkdirSync('samples/fixtures', {recursive: true})
for (const file of ['ILens.Patterns.dll', 'ILens.Dependency.dll']) copyFileSync(`samples/bin/${file}`, `samples/fixtures/${file}`)
// The start page's example: small and readable, compiled for inspection only.
const example = spawnSync('dotnet', ['build', 'samples/Example', '-c', 'Release', '-o', 'samples/bin'], {stdio: 'inherit'})
if (example.status !== 0) process.exit(example.status ?? 1)
copyFileSync('samples/bin/ILens.Example.dll', 'samples/fixtures/ILens.Example.dll')
copyFileSync('samples/fixtures/ILens.Example.dll', 'public/samples/ILens.Example.dll')

// A packer shape: a managed payload carried in an embedded resource.
const packed = spawnSync('dotnet', ['build', 'samples/Packed', '-c', 'Release', '-o', 'samples/bin'], {stdio: 'inherit'})
if (packed.status !== 0) process.exit(packed.status ?? 1)
copyFileSync('samples/bin/ILens.Packed.dll', 'samples/fixtures/ILens.Packed.dll')

// Additional write-verification fixtures; compiled for inspection, never executed.
const deobfuscation = spawnSync('dotnet', ['build', 'samples/Deobfuscation', '-c', 'Release', '-o', 'samples/bin'], {stdio: 'inherit'})
if (deobfuscation.status !== 0) process.exit(deobfuscation.status ?? 1)
copyFileSync('samples/bin/ILens.Deobfuscation.dll', 'samples/fixtures/ILens.Deobfuscation.dll')

const proxies = spawnSync('dotnet', ['build', 'samples/Proxies', '-c', 'Release', '-o', 'samples/bin'], {stdio: 'inherit'})
if (proxies.status !== 0) process.exit(proxies.status ?? 1)
copyFileSync('samples/bin/ILens.Proxies.dll', 'samples/fixtures/ILens.Proxies.dll')

// Array-backed decoders; the Field-RVA layout forces the same-size literal writer.
const arrayStrings = spawnSync('dotnet', ['build', 'samples/ArrayStrings', '-c', 'Release', '-o', 'samples/bin'], {stdio: 'inherit'})
if (arrayStrings.status !== 0) process.exit(arrayStrings.status ?? 1)
copyFileSync('samples/bin/ILens.ArrayStrings.dll', 'samples/fixtures/ILens.ArrayStrings.dll')

const editing = spawnSync('dotnet', ['build', 'samples/Editing', '-c', 'Release', '-o', 'samples/bin'], {stdio: 'inherit'})
if (editing.status !== 0) process.exit(editing.status ?? 1)
copyFileSync('samples/bin/ILens.Editing.dll', 'samples/fixtures/ILens.Editing.dll')


// Fixed-offset string rewriting with EH and compiler-generated Field-RVA storage.
const resourceStrings = spawnSync('dotnet', ['build', 'samples/ResourceStrings', '-c', 'Release', '-o', 'samples/bin'], {stdio: 'inherit'})
if (resourceStrings.status !== 0) process.exit(resourceStrings.status ?? 1)
copyFileSync('samples/bin/ILens.ResourceStrings.dll', 'samples/fixtures/ILens.ResourceStrings.dll')

// C# language constructs for reconstruction tests; compiled only, never executed.
const language = spawnSync('dotnet', ['build', 'samples/Language', '-c', 'Release', '-o', 'samples/bin'], {stdio: 'inherit'})
if (language.status !== 0) process.exit(language.status ?? 1)
copyFileSync('samples/bin/ILens.Language.dll', 'samples/fixtures/ILens.Language.dll')

// Source-readability regression fixtures, also compiled only for static inspection.
const readability = spawnSync('dotnet', ['build', 'samples/Readability', '-c', 'Release', '-o', 'samples/bin'], {stdio: 'inherit'})
if (readability.status !== 0) process.exit(readability.status ?? 1)
copyFileSync('samples/bin/ILens.Readability.dll', 'samples/fixtures/ILens.Readability.dll')

// Synthetic watermark metadata, not assemblies processed by an obfuscator.
for (const [name, properties] of [
  ['ILens.InspectionMarkers', []],
  ['ILens.NoTarget', ['-p:DefineConstants=NO_MARKERS', '-p:GenerateTargetFrameworkAttribute=false']],
]) {
  const build = spawnSync('dotnet', ['build', 'samples/InspectionMarkers', '-c', 'Release', '-o', 'samples/bin', `-p:AssemblyName=${name}`, ...properties], {stdio: 'inherit'})
  if (build.status !== 0) process.exit(build.status ?? 1)
  copyFileSync(`samples/bin/${name}.dll`, `samples/fixtures/${name}.dll`)
}

// Cross-assembly inspection and .resources fixtures; compile only.
const workspace = spawnSync('dotnet', ['build', 'samples/Workspace', '-c', 'Release', '-o', 'samples/bin'], {stdio:'inherit'})
if (workspace.status !== 0) process.exit(workspace.status ?? 1)
for (const name of ['ILens.Workspace.dll', 'ILens.WorkspaceDependency.dll']) copyFileSync(`samples/bin/${name}`, `samples/fixtures/${name}`)

// Reference-only API fixture from the same owned source, with debug paths disabled.
copyFileSync('samples/WorkspaceDependency/obj/Release/net10.0/ref/ILens.WorkspaceDependency.dll', 'samples/fixtures/ILens.WorkspaceDependency.ref.dll')

const automatic = spawnSync('dotnet', ['build', 'samples/AutomaticDependencies', '-c', 'Release', '-o', 'samples/bin'], {stdio:'inherit'})
if (automatic.status !== 0) process.exit(automatic.status ?? 1)
// OutputType=Exe supplies a real managed entry point. The SDK uses .dll for the
// filename by default; distribute this fixture as .exe to cover the EXE picker.
copyFileSync('samples/bin/ILens.AutomaticDependencies.dll', 'samples/fixtures/ILens.AutomaticDependencies.exe')

// A C# method edit compiled the way the browser compiler does (Roslyn, same options):
// the merge test maps it back into ILens.Editing. Compiled only, never executed.
{
  const harness = spawnSync('dotnet', ['build', 'samples/CSharpEditHarness', '-c', 'Release', '-o', 'samples/bin/harness'], {stdio: 'inherit'})
  if (harness.status !== 0) process.exit(harness.status ?? 1)
  const sdks = spawnSync('dotnet', ['--list-sdks'], {encoding: 'utf8'}).stdout
  const root = /^10\.[^\n[]+\[(.+)\]/m.exec(sdks)?.[1]
  const packs = path.join(root, '..', 'packs', 'Microsoft.NETCore.App.Ref')
  const version = readdirSync(packs).filter(v => v.startsWith('10.')).sort().at(-1)
  const references = readdirSync(path.join(packs, version, 'ref', 'net10.0')).filter(f => f.endsWith('.dll')).map(f => path.join(packs, version, 'ref', 'net10.0', f))
  const compile = spawnSync('dotnet', ['samples/bin/harness/CSharpEditHarness.dll', 'samples/CSharpEditHarness/Increase.edit.cs', 'samples/fixtures/ILens.Editing.csharp-edit.dll', 'samples/fixtures/ILens.Editing.dll', ...references], {stdio: 'inherit'})
  if (compile.status !== 0) process.exit(compile.status ?? 1)
}
