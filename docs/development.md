# Development

## Toolchain and installation

`.nvmrc` pins Node.js 22.23.1. `rust-toolchain.toml` pins Rust 1.92.0, rustfmt, Clippy, and `wasm32-unknown-unknown`. Cargo and npm lockfiles are committed. On a native machine, install rustup and normal compiler/linker tools for your OS before running `npm ci` and `npm run dev`.

`.npmrc` preserves the tested peer-resolution behavior for the pinned Rari dependency graph. Use `npm ci` for a clean installation. The .NET 10 SDK is optional and only needed for fixture regeneration.

Only two helper scripts are required:

| Script | Purpose |
| --- | --- |
| `scripts/build-wasm.mjs` | Install matching wasm-bindgen-cli when missing, build Rust/WASM, and generate browser bindings |
| `scripts/build-fixtures.mjs` | Compile the C# fixtures and copy their DLLs into the checked-in fixture/sample locations |

One-time generators and the redundant Rari launcher were removed. npm invokes Rari's packaged CLI directly with Node because Rari 0.15.17's CLI entry has no shebang. `build:app` is the application-only step used after Docker or CI has already built WASM; normal development should use `npm run build`.

## Ports and browser integration

`npm run dev` serves Vite on port 5173, with Rari's internal backend on 3000. Open 5173 during development. `npm start` serves the production application on 3000. Do not run development and production servers together on these default ports.

Restart `npm run dev` after changing Rust or worker code. The build step regenerates bindings before Rari starts. Worker source is excluded from Rari component HMR to prevent it being registered as a server component.

Global styling is imported as a CSS module by the root layout, using global selectors. The Vite development HTML also links that stylesheet. See [CSS integration](css-loading.md) before changing either entry point. CodeMirror themes use its supported theme API. The production CSP permits browser WASM compilation.

## Codespaces and Dev Containers

`.devcontainer/devcontainer.json` provides Node.js, Rust, compiler dependencies, and the WASM target. Its creation command runs `npm ci` and `npm run build:wasm`; it does not create a server, commit, or upload source. The first setup can take several minutes. Run `npm run dev` when setup completes and open forwarded port 5173.

The development server binds all interfaces inside the container through `ILENS_DEV_HOST`. In Codespaces, Vite allows the exact hostname derived from `CODESPACE_NAME` and `GITHUB_CODESPACES_PORT_FORWARDING_DOMAIN`. Local development keeps a loopback binding. RSC and asset requests use the same browser origin; there is no need to expose the internal Rari port separately.

Keep forwarded ports private while reviewing the application. The .NET SDK and Chromium are not included in container setup; install them only if you need to regenerate fixtures or run browser tests. Chromium can be installed with `npx playwright install --with-deps chromium`.

References: [GitHub's Node.js container setup](https://docs.github.com/en/codespaces/setting-up-your-project-for-codespaces/adding-a-dev-container-configuration/setting-up-your-nodejs-project-for-codespaces), [Codespaces environment variables](https://docs.github.com/en/codespaces/developing-in-a-codespace/default-environment-variables-for-your-codespace), [port forwarding](https://docs.github.com/en/codespaces/developing-in-a-codespace/forwarding-ports-in-your-codespace).

## Docker

The Dockerfile builds the Rust core with Rust 1.92.0 and wasm-bindgen 0.2.105, builds the Rari application with Node.js 22.23.1, and copies the production assets/dependencies into the runtime image. Rust tooling and source are excluded from that runtime image. The image uses Debian Trixie for the native Rari runtime and supports the platform selected by Docker.

The runtime invokes Rari's packaged native server directly with `--host 0.0.0.0`; the framework's npm launcher otherwise binds loopback outside its named hosting platforms. The image listens on container port 3000 and runs as the `node` user. Its health check requests the application root. Compose adds an init process, restart policy, read-only root filesystem, and a writable temporary directory. No persistent volume is needed.

```sh
docker compose up --build -d --wait
docker compose logs -f app
docker compose down
```

Compose defaults to `127.0.0.1:3000`. Copy `.env.example` to `.env` if you want persistent local overrides, or set `ILENS_PORT` and `ILENS_BIND_ADDRESS` on the command line. The `.env` file is ignored by Git and Docker. These settings control serving only; assembly analysis remains inside the browser.

A remote installation should use HTTPS, normally through a reverse proxy. Map the host binding and port deliberately for that installation. The repository provides no automatic deployment or registry publishing.

## Tests and CI

GitHub Actions runs on pushes, pull requests, and manual dispatch. Actions are pinned to reviewed commit SHAs; the workflow token has read-only repository access. Overlapping runs for the same ref are cancelled. Native checks use the pinned toolchain and lockfiles, and browser checks exercise actual DLL parsing through WASM.

Development and production Playwright results are kept separately in `test-results/dev` and `test-results/production`. HTML reports live in `playwright-report`. CI retains browser diagnostics on failure for seven days. These directories are ignored by Git and Docker.

The production browser config accepts `PLAYWRIGHT_BASE_URL` to test an already running Docker or production server without launching another server:

```sh
ILENS_PORT=3100 docker compose up --build -d --wait
PLAYWRIGHT_BASE_URL=http://localhost:3100 npm run test:production
```

The fixture source covers arithmetic, branches, loops, switches, exceptions, generics, properties, arrays, delegates, async and iterator bodies. Separate marker fixtures verify metadata-based obfuscation inspection; they are intentionally labeled synthetic markers, not samples actually protected by those tools.

`samples/Directory.Build.props` disables PDB generation for distributed fixtures and maps source paths to `/_/ILens`. This also applies to direct `dotnet build` commands. The Rust fixture privacy tests reject debug symbol records and local user-directory paths in every checked-in DLL, including the public sample. Never copy local PDB files into the repository or the public assets.

## Screenshots

Documentation images are actual application captures, not mockups. The styling and settings tests open the checked-in DLLs, wait for reconstructed output, verify computed CSS, and take screenshots at 1512 × 982.

```sh
npm run build
npx playwright test --config playwright.production.config.ts tests/e2e/styles.spec.ts tests/e2e/settings-inspection.spec.ts
```

Copy the selected images from `test-results/production` into `docs/screenshots`, using the names linked in the README. Keep only the final captures in documentation. Temporary traces, failure captures, and HTML reports remain ignored and can be deleted after review.
