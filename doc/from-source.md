# Running from source

How to build and run xherd from a clone of this repository: the Docker image, or a native binary. Use it to try a branch that is not released, to run on macOS natively (no macOS binaries are published yet), or as the first step before [changing xherd](development.md). To install a published release instead, see [Getting started](getting-started.md).

## Prerequisites

| | Docker build | Native build |
|---|---|---|
| Build | A container engine with Compose (Docker Desktop, OrbStack, Colima, Docker on Linux) | Rust (edition 2024, stable), Node and pnpm |
| Run | Nothing else: everything is in the image | `git`, `tmux` ≥ 3.3, Claude Code logged in, `gh` / `glab` on `PATH` |

On macOS with Homebrew's keg-only rustup, put it on `PATH` first: `export PATH=/opt/homebrew/opt/rustup/bin:$PATH`.

## Docker, built from the repository

`deploy/compose/compose.yaml` builds the image from this repository (`deploy/docker/Dockerfile`) and runs it as `xherd:dev`. Its neighbour `compose.release.yaml` is the end-user file that runs the published image instead.

```sh
cd deploy/compose
printf 'XHERD_WORKSPACE=%s/xherd\nXHERD_UID=%s\nXHERD_GID=%s\n' "$HOME" "$(id -u)" "$(id -g)" > .env
mkdir -p ~/xherd
docker compose up -d --build        # or: docker-compose up -d --build
```

Open <http://127.0.0.1:4740/>. `.env` is gitignored; `.env.example` lists the variables:

- `XHERD_WORKSPACE`: absolute path, mounted at the **same path** in the container, so git worktrees stay valid on both sides and your IDE opens them directly.
- `XHERD_UID` / `XHERD_GID`: your ids; agents write files as you.
- `XHERD_PORT` (optional, default 4740): the port published on your machine's loopback.

The Dockerfile is multi-stage: the UI (pnpm), the daemon (cargo), and the runtime on Debian trixie with `git`, `tmux`, `gh`, `glab`, `socat`, `tini`, Node and Claude Code, `uv`, Go, and seeds of SDKMAN, nvm and rustup. Version pins are `ARG`s at the top of the runtime stage.

After a change, rebuild and recreate with the same `docker compose up -d --build` (pause the agents first if you care about their running turns). `xherd docker doctor` and `xherd docker update` work only on the compose file in `~/.config/xherd` (`$XDG_CONFIG_HOME/xherd`), not on this one. The image has no `.git`, so it shows the Cargo version unless you pass the build argument:

```sh
docker compose build --build-arg XHERD_VERSION="$(git describe --tags --always)"
```

To build the image alone:

```sh
docker build -f deploy/docker/Dockerfile -t xherd:dev .
```

Topology, the two users inside the container, volumes and git access are in [Docker](docker.md).

## Native build

The release binary embeds the web UI at compile time, so build the UI first:

```sh
(cd ui && pnpm install && pnpm build)          # ui/dist
cargo build --release                          # target/release/xherd
./target/release/xherd daemon                  # JSON logs on stderr; XHERD_LOG=debug for more
./target/release/xherd install                 # once: the /xherd:* commands for Claude Code
```

Put `target/release/xherd` on your `PATH` (the agents' hooks call it by its absolute path, so do not move it while agents exist). Open <http://127.0.0.1:4740/>.

- **Data root**: `XHERD_HOME`, else `$XDG_DATA_HOME/xherd`, else `~/.local/share/xherd`. `config.json` there holds the port (4740), the tmux socket and the user token the CLI uses. Point `XHERD_HOME` at a throwaway folder to try things without touching your real setup.
- **Workspace**: `config.workspace`, else `$XHERD_WORKSPACE`, else `~/xherd`. Worktrees go to `<workspace>/worktrees/<repo>/<agent>`, cloned repositories to `<workspace>/repos/<host>/<path>`.
- **Claude Code**: must be logged in once (`claude`, then `/login`). xherd marks each new worktree trusted in Claude Code's config, so no trust step is needed.
- **Toolchains**: natively xherd installs nothing; agents use what is on your `PATH`. Installing from version files happens only in the container.
- Native runs have a single user: agents can read what you can. `xherd security check` lists what that exposes.

### Debug builds and UI work

A debug build serves `ui/dist` from disk instead of embedding it, so after `pnpm build` a page reload is enough:

```sh
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- daemon
XHERD_HOME=/tmp/xh cargo run -p xherd-cli -- status
cd ui && pnpm dev            # Vite dev server, proxies /api to XHERD_URL (default 127.0.0.1:4740)
```

`scripts/demo.sh` runs a throwaway daemon on port 4741 with fake agents, for screenshots ([Development](development.md#demo-data-for-screenshots)).

## Release builds like CI

`.github/workflows/release.yml` builds the published artefacts; to reproduce them locally:

- **Linux binaries**: static musl with `cargo zigbuild --release --locked -p xherd-cli --target x86_64-unknown-linux-musl` (or `aarch64-…`), with `XHERD_VERSION` set and `ui/dist` built first.
- **Packages**: `.deb` / `.rpm` from `deploy/nfpm/nfpm.yaml` (`VERSION=… ARCH=amd64 nfpm package -f deploy/nfpm/nfpm.yaml -p deb -t dist/`, the binary at `dist/pkg/xherd`).
- **Image**: the same Dockerfile with `--build-arg XHERD_VERSION=…`.
- **Compose file**: `deploy/compose/compose.release.yaml`, attached as `compose.yaml`.

How to cut a release is in [Development](development.md#releases).

## Next

- [Getting started](getting-started.md#first-run): the first run and a first agent, the same from source
- [Development](development.md): tests, conventions, how to add things
- [Architecture](architecture.md)
