# Getting started

How to install xherd from a published release, start it, and run a first job. xherd runs in Docker or natively, and neither is the default: see [Choose how to run it](#choose-how-to-run-it). See [Docker](docker.md) for details and troubleshooting.

To build xherd yourself (an unreleased branch, or to change it), see [Running from source](../dev/from-source.md).

## Choose how to run it

Docker and native are equal options; pick by what your jobs need.

- **Pick Docker if** you want jobs kept away from your home directory, keys and other projects, and your repositories build on Linux.
- **Pick native if** you build iOS/macOS apps, need an emulator or simulator, run Docker from your repositories, or want your own tools and shell setup.

On a Mac, either works: Docker, or `brew install xionjames/tap/xherd` (see [Run natively](#run-natively)). xherd is a single-user tool: one person, one daemon, one workspace folder.

### Docker

Pros:

- **Isolation.** Jobs see only the workspace folder you share, and the daemon's secrets live under a separate user.
- **Nothing to install** but a container engine with Compose (Docker Desktop, OrbStack, Colima or Docker on Linux). `git`, `tmux`, `gh`, `glab` and the toolchain managers come in the image; the first start downloads the coding-agent CLIs at checksum-verified versions, so give it a minute and network access.
- **Same setup on every machine**, easy to update (pull a new image) or throw away (rebuild the container).
- **The `all` permission mode**, which needs this isolation. Your IDE still works: the workspace is mounted at the same path inside and outside.

Cons:

- Linux only inside: no Xcode (so no iOS, macOS or watchOS builds, no Apple simulators), no Android or other emulators, no GPU-dependent work.
- No Docker-in-Docker: repositories that run containers (Testcontainers, `docker compose`) do not work.
- Toolchains only through the bundled managers (Java, Node, Go, Rust, Python).
- On macOS, file access on the shared workspace can be slower, so large builds and installs may take longer.
- No desktop notifications; SSH agent forwarding does not work on Colima.

### Native (macOS, Linux)

Pros: every tool of your machine (Xcode, simulators, emulators, Docker, GPU, your shell and credentials), native file speed, no container engine.

Cons:

- No isolation: jobs run as you and can read your home, keys and other projects. xherd's secrets are a guardrail, not a boundary.
- You install and update the coding-agent CLI, `git`, `tmux` ≥ 3.3, `gh` / `glab` and toolchains yourself.
- The `all` permission mode is not available.

## Versions

Releases are listed at <https://github.com/xionjames/xherd/releases>. Each one publishes the image `ghcr.io/xionjames/xherd` (amd64 and arm64, tagged `latest`, `X.Y` and `X.Y.Z`), the Linux binaries (`.tar.gz`, `.deb`, `.rpm`) and the macOS binaries (`.tar.gz`, Apple silicon and Intel), each with a `.sha256` file. Stable releases are also in the Homebrew tap `xionjames/tap`.

## Run with Docker

You need only a container engine with Compose. Nothing else goes on your machine.

### 1. Get the compose file

Make a folder for it (any name; `~/xherd-docker` here) and the workspace your jobs will work in, then download the compose file of the latest release and write your values beside it:

```sh
mkdir -p ~/xherd-docker ~/xherd
cd ~/xherd-docker
curl -fLO https://github.com/xionjames/xherd/releases/latest/download/compose.yaml
printf 'XHERD_WORKSPACE=%s/xherd\nXHERD_UID=%s\nXHERD_GID=%s\n' "$HOME" "$(id -u)" "$(id -g)" > .env
```

The `.env` values:

- `XHERD_WORKSPACE` must be an absolute path. It is mounted at the **same path** inside the container, so git worktrees stay valid on both sides and your IDE can open them directly. Repositories the jobs work on are cloned into it.
- `XHERD_UID` / `XHERD_GID` are your ids: jobs write files as you.
- `XHERD_PORT` (optional, default 4740) changes the port on your machine.
- `XHERD_IMAGE` (optional, default `ghcr.io/xionjames/xherd:latest`) stays on one version, for example `ghcr.io/xionjames/xherd:0.1.0`.

To forward your SSH job (Docker Desktop, OrbStack), uncomment the marked line in `compose.yaml`.

### 2. Start it

```sh
docker compose up -d        # or: docker-compose up -d
```

Open <http://127.0.0.1:4740/>. The port is published on your machine's loopback only, so nothing on your network can reach it. The [Welcome](#first-run) screen opens.

### Everyday operation

Run these in the folder of `compose.yaml`.

| Task | How |
|---|---|
| Logs | `docker compose logs -f` |
| Restart | `docker compose restart`: jobs waiting at a gate are parked, the others resumed |
| Stop | `docker compose down` (the volumes and the workspace stay) |
| `xherd` commands | `docker exec -u xherd-daemon xherd xherd ls`, see [Everyday CLI](guide.md#everyday-cli) |

### Update

Pause your jobs (the **Pause all** button on the Jobs page), then:

```sh
docker compose pull && docker compose up -d
```

and resume them. Your configuration, logins, toolchains and jobs live in the volumes and the workspace, so they survive the new container. Take a backup first if you care about the jobs' state: Settings → Backups.

## Run natively

### macOS

```sh
brew install xionjames/tap/xherd      # also installs git and tmux
```

Or download `xherd-<version>-aarch64-apple-darwin.tar.gz` (Apple silicon) or `xherd-<version>-x86_64-apple-darwin.tar.gz` (Intel) from the [releases page](https://github.com/xionjames/xherd/releases), check it (`shasum -a 256 -c <file>.sha256`) and install the binary:

```sh
tar -xzf xherd-<version>-aarch64-apple-darwin.tar.gz \
  && sudo install xherd-<version>-aarch64-apple-darwin/xherd /usr/local/bin/
brew install git tmux                 # tmux ≥ 3.3
```

The binaries are not signed or notarized. Homebrew and `curl` downloads are not quarantined; a browser download is, so macOS refuses to open it until you run `xattr -d com.apple.quarantine /usr/local/bin/xherd`.

### Linux

Download the package for your machine from the [releases page](https://github.com/xionjames/xherd/releases) and check it against its `.sha256` file (`sha256sum -c <file>.sha256`), then install one of:

```sh
sudo apt install ./xherd_<version>_amd64.deb                      # Debian, Ubuntu
sudo dnf install ./xherd-<version>-1.x86_64.rpm                   # Fedora, RHEL
tar -xzf xherd-<version>-x86_64-unknown-linux-musl.tar.gz \
  && sudo install xherd-<version>-x86_64-unknown-linux-musl/xherd /usr/local/bin/   # any Linux
```

(arm64: `arm64` / `aarch64` in the names.) The binary is static and has the web UI built in. The packages pull in `git` and `tmux`; with the tarball, install them yourself (`tmux` ≥ 3.3).

You also need:

- A runtime: **Claude Code** logged in (`claude`, then `/login`, once), or opencode, or Codex (`codex login`), or Pi (`pi`, then `/login`).
- **`gh`** (GitHub) or **`glab`** (GitLab) for pull requests, logged in. Settings → Git access can also hold a token per host.
- The toolchains your repositories need: natively xherd uses what is on your `PATH` and installs nothing.

Start the daemon. It installs the xherd commands (`/xherd:*`) for each runtime that is on, at every start, so there is nothing else to run:

```sh
xherd daemon            # keep it running (a terminal, tmux, or your own service)
```

Open <http://127.0.0.1:4740/>.

Data lives in `XHERD_HOME`, else `$XDG_DATA_HOME/xherd`, else `~/.local/share/xherd` (`config.json` there holds the port and the token the CLI uses). Worktrees go to `<workspace>/worktrees/<repo>/<job>`; the workspace is `config.workspace`, else `$XHERD_WORKSPACE`, else `~/xherd`.

To update, install the newer package (macOS with Homebrew: `brew upgrade xherd`) and restart `xherd daemon`. The restart brings the commands and the live jobs' hook files up to date; no `xherd install` is needed. If you edited a command file, xherd keeps your version and Setup asks what to do (see [Setup checks](guide.md#welcome-plugins-and-git-access)).

## First run

1. **Welcome** opens by itself the first time. It walks you through the plugins (which runtime, git hosts and trackers you use, and their logins, in a terminal in the browser), your commit identity, your git hosts (HTTPS tokens or SSH), and the setup checks. Everything it sets is in Settings later.
2. **Setup checks** must pass before you can create jobs. Each failing check shows its fix.
3. **Create a job** from the UI (**New job**) or the CLI:

   ```sh
   xherd new passkeys --ticket PROJ-123 --repo https://github.com/you/project "Add login with passkeys"
   ```

   `--repo` takes a URL (cloned once into `<workspace>/repos/`) or a local path (in Docker, one inside the workspace). The job writes a spec and stops for your approval: the [user guide](guide.md) takes it from there.

## Next

- [Using xherd](guide.md): gates and reviews, autonomy, pull requests, TODOs, notifications, the everyday CLI
- [Docker](docker.md): topology, the two users in the container, git access, toolchains
- [Security model](security.md)
- [Configuration](../reference/configuration.md) and [CLI reference](../reference/cli.md)
- [Running from source](../dev/from-source.md), to build xherd yourself
