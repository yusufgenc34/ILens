# Contributing

Use the versions in `.nvmrc` and `rust-toolchain.toml`. Start with the [README](README.md) or open the repository in its Dev Container.

## Working on the project

Keep parsing, analysis, transformations, and rendering separate. The Rust core accepts byte buffers and must remain compatible with `wasm32-unknown-unknown`. Browser analysis belongs in the worker; the Rari server only provides the application shell and assets.

Never execute analyzed assemblies or send their contents to a server. A malformed input should produce a structured error. Unsupported C# reconstruction should preserve the IL view and explain the limitation.

For compiler changes, add a small C# fixture or a focused regression case that demonstrates the behavior. Use source you can redistribute. Fixture assemblies are compiled for inspection, never executed by the test suite. Regenerate the existing fixtures with `npm run fixtures` when their source changes.

For interface changes, verify the production build in the browser and update screenshots when the visible behavior changes. Match the existing neutral workspace appearance; editor colors are controlled by settings.

## Validation

```sh
npm ci
npm run format:check
npm test
npm run build:wasm
npm run typecheck
npm run lint
npx playwright install --with-deps chromium
npm run test:e2e
npm run build
npm run test:production
```

For container changes, also build and test Docker:

```sh
ILENS_PORT=3100 docker compose up --build -d --wait
PLAYWRIGHT_BASE_URL=http://localhost:3100 npm run test:production
docker compose down
```

See [development notes](docs/development.md) for ports, worker rebuilds, and screenshot capture.

## Pull requests

Describe the problem, resulting behavior, validation, and known limitations. Include a reproducible input for parser issues and screenshots for visible changes. Keep unrelated changes separate.

Commit source, lockfiles, fixture DLLs, and selected documentation screenshots. Do not commit generated WASM, `node_modules`, `target`, `dist`, C# `bin`/`obj` directories, test reports, environment files, or scratch data.

Use UTF-8 and LF line endings as configured in `.editorconfig` and `.gitattributes`. Run `cargo fmt --all` after Rust edits and keep Clippy warning-free. Changes to the opcode table must retain the attribution in [third-party notices](docs/third-party-notices.md).
