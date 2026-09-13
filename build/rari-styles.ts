import fs from 'node:fs/promises'
import path from 'node:path'
import type {Plugin} from 'vite-plus'

// Rari 0.15's independent SSR bundler cannot import plain CSS and does not
// run Vite/Tailwind transforms for CSS Modules. Let Vite compile the HTML's
// stylesheet entry, then attach the hashed asset to Rari's root route manifest.
export function rariStyles(): Plugin {
  let outDir = ''
  const styles = new Set<string>()
  return {
    name: 'ilens-rari-styles',
    apply: 'build',
    buildStart() {styles.clear()},
    configResolved(config) {outDir = path.resolve(config.root, config.build.outDir)},
    generateBundle(_options, bundle) {
      for (const asset of Object.values(bundle)) if (asset.type === 'asset' && asset.fileName.endsWith('.css')) styles.add(`/${asset.fileName}`)
    },
    closeBundle: {
      order: 'post', sequential: true,
      async handler() {
        if (!styles.size) throw new Error('Tailwind did not emit a stylesheet. Refusing an unstyled build.')
        const file = path.join(outDir, 'server/routes.json')
        const routes = JSON.parse(await fs.readFile(file, 'utf8')) as {layouts: {filePath: string; componentId: string; css?: string[]}[]}
        const root = routes.layouts.find(layout => /^\/?layout\.tsx$/.test(layout.filePath))
        if (!root) throw new Error('Rari root layout was not found for global styles.')
        root.css = [...new Set([...(root.css ?? []), ...styles])]
        await fs.writeFile(file, JSON.stringify(routes))
        const manifestFile = path.join(outDir, 'server/manifest.json')
        const manifest = JSON.parse(await fs.readFile(manifestFile, 'utf8'))
        if (!manifest.components?.[root.componentId]) throw new Error('Rari root component manifest is missing.')
        manifest.components[root.componentId].css = root.css
        await fs.writeFile(manifestFile, JSON.stringify(manifest))
      },
    },
  }
}
