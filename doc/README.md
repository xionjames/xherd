# xherd documentation

What xherd is and what it looks like: the [project README](../README.md).

## Using xherd

- [Getting started](user/getting-started.md): install a release (Docker recommended, or a Linux package), the first run, a first agent
- [Using xherd](user/guide.md): the screens, gates and reviews, autonomy, pull requests, tickets, TODOs, notifications, backups, the everyday CLI
- [Docker](user/docker.md): how the container is laid out, its two users, git access, toolchains, operating it
- [Security model](user/security.md): what xherd protects, what it does not, how to check your setup

## Reference

- [CLI](reference/cli.md): every `xherd` command, and the environment variables
- [Configuration](reference/configuration.md): `config.json`, repo profiles, the per-agent files
- [HTTP API](reference/http-api.md): routes, authentication, WebSockets
- [XAP](reference/xap.md): the protocol agents and SDDs use to talk to xherd

## Extending xherd

- [Plugins](extend/plugins.md): runtimes, git hosts, issue trackers, toolchains; the manifest and the command protocol
- [Flow packs](extend/flow-packs.md): stages, gates and the prompts that drive them
- [Add xherd support to your SDD](extend/integrate-your-sdd.md): run your own spec-driven method under xherd (partly planned)

## Working on xherd

- [Running from source](dev/from-source.md): build the image or the binary from this repository
- [Development](dev/development.md): prerequisites, build and test, conventions, common changes, releases, writing docs
- [Architecture](dev/architecture.md): crates, actors, gates, sessions, plugins, guardrails, storage
- [`AGENTS.md`](../AGENTS.md): working notes for coding agents (status per feature, gotchas)
- [Roadmap](../ROADMAP.md): what is done and what is left, by theme

## Working documents

[Product specification](internal/spec.md) and [design](internal/design.md): requirements (`R-xxx`), design decisions and milestones. They predate the pages above, describe planned work too, and will be retired as the pages above cover them.
