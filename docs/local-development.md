# Local Development

MundusX is a mixed Rust and Node repository. The Rust workspace builds the
`opengpu`, `mundusx`, node-agent, tray, installer, and local agent-server
binaries. The Node workspace currently builds the static public docs/install
preview.

## Prerequisites

- Rust through `rustup`; this repository pins the stable toolchain in
  `rust-toolchain.toml`.
- Node.js and pnpm. The root `package.json` declares `pnpm@9.15.9`.
- Python 3 when running the localhost smoke test, because it starts a local
  static file server.

On a new macOS machine, install Rust directly with rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --profile minimal
. "$HOME/.cargo/env"
rustup component add rustfmt clippy
```

Homebrew no longer supports Intel macOS as a primary platform as of September
2026, so do not use `brew install rustup-init` on Intel Macs. If direct rustup
installation is blocked by local policy, use MacPorts instead:

```bash
sudo port install rust cargo
```

Restart the shell after installing Rust if `cargo` is still not on `PATH`.

## Build

```bash
cargo build --workspace
npm run build:docs-site
```

The docs build writes a static preview to `dist/public-docs-site`.

## Run The CLI Locally

The main local binaries can be run directly through Cargo:

```bash
cargo run -p opengpu --bin opengpu -- --help
cargo run -p opengpu --bin mundusx -- --help
cargo run -p opengpu-node-agent -- --help
```

Use an isolated config directory when testing onboarding or node state:

```bash
OPENGPU_HOME=/tmp/opengpu-dev cargo run -p opengpu --bin opengpu -- status
MUNDUSX_HOME=/tmp/mundusx-dev cargo run -p opengpu --bin mundusx -- sessions
```

## Run The Local Agent API

`mundusx-agent-server` binds to loopback only by default:

```bash
MUNDUSX_HOME=/tmp/mundusx-dev cargo run -p mundusx-agent-server -- --workspace .
```

The API listens on `http://127.0.0.1:11436` and exposes
`/v1/chat/completions`. Set `MUNDUSX_AGENT_API_KEY` when another local app
should authenticate to it.

## Preview The Docs And Install Site

Build the static site:

```bash
npm run build:docs-site
```

Serve it locally:

```bash
python3 -m http.server 3002 --bind 127.0.0.1 --directory dist/public-docs-site
```

Then open:

- `http://127.0.0.1:3002/docs`
- `http://127.0.0.1:3002/install`
- `http://127.0.0.1:3002/public/install`

## Local Release Preview And Smoke Test

The release preview helper builds localhost-only release artifacts and serves
them from `http://127.0.0.1:8788/releases/latest/download/`:

```bash
scripts/local-release-preview.sh up
scripts/local-release-preview.sh verify
```

The one-shot smoke test checks the docs/install site and local release preview:

```bash
SMOKE_SKIP_CONTROL_PLANE=1 npm run smoke:local
```

Leave `SMOKE_SKIP_CONTROL_PLANE` unset only when a compatible control-plane
server is already running at `http://127.0.0.1:8787`.
