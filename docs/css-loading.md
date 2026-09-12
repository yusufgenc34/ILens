# CSS loading diagnosis and regression coverage

The unstyled appearance was reproduced with Chromium: existing `.application`, `.workspace`, `.toolbar` and pane classes were present, but the application computed to the browser's Times font, transparent background and block layout.

The original `index.html` imported global CSS through an inline module script, following the generated Rari template. Vite's port 5173 transformed that import and injected CSS, but Rari's root layout had no stylesheet dependency. The Rari route stylesheet manifest was empty. Its internal development backend on port 3000 therefore rendered server HTML without effective styles. The presence of rendered React content was not evidence of a successful UI build.

A direct plain `.css` import in the root layout exposed an additional build problem: Rari 0.15.17's server component bundler passed it to Rolldown, which rejects direct CSS bundling. Rari does explicitly support CSS Modules through Lightning CSS, collecting extracted CSS into the route manifest.

The fix uses that supported path:

1. `src/app/layout.tsx` imports `./globals.module.css`.
2. Global selectors are marked `:global(...)`, preserving every existing DOM class and the existing design.
3. Rari emits `/assets/server/<hash>.css` and lists it in `dist/server/routes.json` for the root layout.
4. `index.html` links the same stylesheet for Vite's development entry. Vite transforms it into a real CSS response and rewrites the link for production.
5. No inline-style replacement, Tailwind installation, or redesign was introduced. CodeMirror's existing theme extension remains responsible for its editor-specific styles. The default Neutral editor theme uses neutral syntax tones, contrast and weight. Settings optionally select a separate code palette without changing the workspace CSS.

Use **localhost:5173** for `npm run dev`. Port 3000 is an internal backend during development, not the Vite application URL. After `npm run build`, `npm start` serves the complete production application on **localhost:3000**.

`tests/e2e/styles.spec.ts` verifies successful CSS responses with `text/css`, no browser runtime errors, sans-serif typography, flex toolbar/application, a three-pane grid with measured pane positions, styled buttons, a real WASM-decompiled method, editor gutters/monospace font, and both theme backgrounds. It captures start, dark-method and light-method screenshots. The same test runs against development and production; `playwright.production.config.ts` starts the real production server. Production browser testing additionally caught and fixed the missing `wasm-unsafe-eval` CSP permission.

Run `npm run test:e2e` and, after building, `npm run test:production`. Screenshots are written under `test-results/` by Playwright. A successful HTML response alone is intentionally insufficient.
