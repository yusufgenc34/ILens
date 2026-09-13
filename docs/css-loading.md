# Tailwind and Rari stylesheet integration

The interface uses Tailwind CSS **4.3.3**, compiled by the matching `@tailwindcss/vite` plugin. Layout, spacing, typography, controls, states and responsive rules are utility classes in the React components. `src/lib/ui.ts` shares Tailwind recipes for repeated buttons and dialogs; `cn` uses `tailwind-merge` to resolve variants. Semantic DOM names such as `workspace` remain as navigation/test hooks, not CSS layout definitions.

`src/app/globals.css` contains the Tailwind import, explicit component/recipe scanning, semantic theme tokens, document defaults, dark/light palettes and the warning animation. The previous 1,351-line `globals.module.css` has been removed. CodeMirror's supported theme API still handles code highlighting and its independent editor palettes. Production ships compiled CSS; no Tailwind CDN or browser compiler is used. No PostCSS plugin or Tailwind v3 `content` configuration is needed with this Vite integration.

## Entry points

1. `index.html` links `/src/app/globals.css`. Vite serves its compiled CSS before development hydration.
2. Vite's production build emits the stylesheet with a content hash under `/assets`.
3. `build/rari-styles.ts` attaches the emitted asset to the root layout in both Rari's route and component manifests. Rari then includes the link in server-rendered HTML and route navigation.
4. The build fails if CSS was not emitted or the expected root manifests are missing. There is no manual CSS copying step.

This adapter is needed for the pinned Rari **0.15.17**: its independent server-component bundler rejects direct plain-CSS imports and its CSS Module extractor bypasses Vite's Tailwind transformation. Importing `globals.css` into a client component also fails that component's independent SSR build. Keeping the CSS entry in Vite and explicitly registering its output avoids these paths. Check the adapter and browser tests when upgrading Rari.

## Regression coverage

`tests/e2e/styles.spec.ts` inspects the delivered HTML for stylesheet links and fetches them with the browser's `Accept: text/css` header. That header matters in Vite development, where CSS module requests with a generic Accept header can return a JavaScript wrapper. Actual browser CSS responses must succeed with a CSS content type.

The tests also verify computed typography, Tailwind classes, pane geometry, the single top menu, code highlighting and dark/light appearance. IL editing is available only in the IL view. Project export tests check centered dialogs and reachable footer actions on shorter viewports. Both development and production configurations capture the real rendered interface and exercise the real WASM parser.

Use **localhost:5173** for `npm run dev`; port 3000 is its internal Rari backend. After building, `npm start` serves production on **localhost:3000**. The production CSP permits WASM compilation. Run `npm run test:e2e` and `npm run test:production`; HTML rendering alone is not sufficient validation.

References: [Tailwind with Vite](https://tailwindcss.com/docs/installation/using-vite), [Tailwind source detection](https://tailwindcss.com/docs/detecting-classes-in-source-files), [Rari project conventions](https://rari.build/docs/getting-started).

## Static Pages entry

`pages/main.tsx` imports `src/app/globals.css` through the standard Vite/Tailwind pipeline. This entry is not processed by Rari's SSR bundler and does not use the `rariStyles` manifest adapter. The default Rari entry still uses the stylesheet link/manifest mechanism described above. Both entries share the same stylesheet and component utilities.
