# GitHub Pages

[Open ILens](https://yusufgenc34.github.io/ILens/).

## Deployment model

The default application uses Rari for its server-rendered shell and Docker deployment. GitHub Pages serves static files and cannot run that server. The separate `pages/main.tsx` entry mounts the same React workspace directly, using the same components, global stylesheet, worker, and Rust/WASM engine. It does not duplicate the decompiler or change the Docker/Rari build.

`vite.pages.config.ts` emits only HTML, CSS, JavaScript, WASM, the favicon, and the repository-owned sample to `dist-pages`. There are no RSC, server-action or application API requests in this target. Future features that require server components must provide a client-only implementation to work on Pages.

The page includes a Content Security Policy in its HTML. Browser WASM compilation and CodeMirror's generated styles are allowed; script, worker and connection origins are restricted to the site. Uploaded assembly bytes still travel only between the browser and its worker.

## Local build and verification

```sh
npm ci
npm run build:pages
npm run preview:pages
```

Open `http://127.0.0.1:4173/ILens/`. The preview command is a local static preview, not a production hosting service.

After stopping a manually started preview, run:

```sh
dotnet tool restore
npm run test:pages
```

The suite starts its own preview and exercises the same explorer, editor, exports, malformed inputs, themes, and IL editing as the Rari tests. A Pages-specific test additionally checks base paths, WASM MIME type, sample loading, stylesheet delivery, and the absence of server endpoints. Export checks require the .NET 10 SDK and Python 3 and only compile or statically verify repository-owned fixture output.

The default base is `/ILens/`. For another repository name or a custom domain, set the same base during both build and preview/test:

```sh
ILENS_PAGES_BASE=/ npm run build:pages
ILENS_PAGES_BASE=/ npm run preview:pages
```

The worker, WASM, favicon, styles and sample URL all follow this base. `dist-pages` is generated and excluded from Git and Docker build context.

## GitHub Actions

Enable **Settings > Pages > Build and deployment > Source: GitHub Actions**. The `Deploy GitHub Pages` workflow runs on pushes to `main` or manual dispatch. It reads the site's actual base path, builds the static application, runs browser tests, and uploads only `dist-pages`. A separate deployment job publishes the verified artifact using the `github-pages` environment.

The build job has read permissions; only the deployment job has Pages and OIDC write permissions. Action versions are pinned to commit SHAs. No personal access token, custom secret, container registry, database, or assembly storage is needed. Forks must enable Pages in their own repository before dispatching this workflow.

The ordinary CI workflow also checks the static target on pull requests. Rari development, production, and Docker checks remain in place. The Pages workflow does not publish Docker images or change release tags.

## References

- [GitHub Pages overview](https://docs.github.com/en/pages/getting-started-with-github-pages/what-is-github-pages)
- [GitHub Pages custom workflows](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages)
- [Vite deployment and base paths](https://vite.dev/guide/static-deploy.html#github-pages)
- [Rari server deployment](https://rari.build/docs/getting-started/deploying)
