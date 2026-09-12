import {spawnSync} from 'node:child_process'
import {copyFileSync, mkdirSync} from 'node:fs'
const result = spawnSync('dotnet', ['build', 'samples/Patterns', '-c', 'Release', '-o', 'samples/bin'], {stdio: 'inherit'})
if (result.status !== 0) process.exit(result.status ?? 1)
mkdirSync('samples/fixtures', {recursive: true})
for (const file of ['ILens.Patterns.dll', 'ILens.Dependency.dll']) copyFileSync(`samples/bin/${file}`, `samples/fixtures/${file}`)
copyFileSync('samples/fixtures/ILens.Patterns.dll', 'public/samples/ILens.Patterns.dll')

// Synthetic watermark metadata, not assemblies processed by an obfuscator.
for (const [name, properties] of [
  ['ILens.InspectionMarkers', []],
  ['ILens.NoTarget', ['-p:DefineConstants=NO_MARKERS', '-p:GenerateTargetFrameworkAttribute=false']],
]) {
  const build = spawnSync('dotnet', ['build', 'samples/InspectionMarkers', '-c', 'Release', '-o', 'samples/bin', `-p:AssemblyName=${name}`, ...properties], {stdio: 'inherit'})
  if (build.status !== 0) process.exit(build.status ?? 1)
  copyFileSync(`samples/bin/${name}.dll`, `samples/fixtures/${name}.dll`)
}
