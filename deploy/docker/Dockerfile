# xherd image (spec R-601, R-651; design §8): the daemon, git, tmux, gh, glab,
# openssh, the agent CLIs and the version managers. Languages are not in the
# image: they are installed into the xherd-home volume on first use.
#
#   docker build -f deploy/docker/Dockerfile -t ghcr.io/xionjames/xherd:dev .
# (plain `docker build` works; releases use `docker buildx` for amd64 + arm64)

ARG RUST_VERSION=1.99
ARG NODE_MAJOR=24

# ---- web UI -------------------------------------------------------------------
FROM node:${NODE_MAJOR}-trixie-slim AS ui
ARG PNPM_VERSION=11.4.0
WORKDIR /src/ui
RUN npm install -g pnpm@${PNPM_VERSION}
COPY ui/package.json ui/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY ui/ ./
RUN pnpm build

# ---- daemon ---------------------------------------------------------------------
FROM rust:${RUST_VERSION}-trixie AS build
WORKDIR /src
COPY . .
COPY --from=ui /src/ui/dist ui/dist
# .git is not in the build context: pass the version shown in the UI
# (`git describe`-style, see crates/xherd-core/build.rs).
ARG XHERD_VERSION=""
ENV XHERD_VERSION=${XHERD_VERSION}
RUN cargo build --release --locked -p xherd-cli \
    && cp target/release/xherd /usr/local/bin/xherd

# ---- runtime --------------------------------------------------------------------
FROM node:${NODE_MAJOR}-trixie-slim AS node
FROM ghcr.io/astral-sh/uv:0.13.7 AS uv

FROM debian:trixie-slim
ARG GO_VERSION=1.27.3
ARG GLAB_VERSION=1.128.0
ARG NVM_VERSION=0.41.0
ARG CLAUDE_CODE_VERSION=2.1.319
ARG OPENCODE_VERSION=1.19.0
ARG CODEX_VERSION=0.167.2

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates curl git tmux openssh-client socat tini bash less procps \
       zip unzip xz-utils jq gnupg \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
       -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
       > /etc/apt/sources.list.d/github-cli.list \
    && apt-get update && apt-get install -y --no-install-recommends gh \
    && curl -fsSL -o /tmp/glab.deb \
       "https://gitlab.com/api/v4/projects/gitlab-org%2Fcli/packages/generic/glab/${GLAB_VERSION}/glab_${GLAB_VERSION}_linux_$(dpkg --print-architecture).deb" \
    && apt-get install -y --no-install-recommends /tmp/glab.deb \
    && rm -rf /tmp/glab.deb /var/lib/apt/lists/*

# Node (for the agent CLIs only; projects get theirs through nvm), Claude Code,
# opencode and Codex.
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
    && npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" \
       "opencode-ai@${OPENCODE_VERSION}" \
       "@openai/codex@${CODEX_VERSION}" \
    && npm cache clean --force

# Version managers (R-532). uv and go are single binaries; SDKMAN, nvm and
# rustup are seeds that `xherd init` copies into the home volume once.
COPY --from=uv /uv /uvx /usr/local/bin/
RUN curl -fsSL "https://go.dev/dl/go${GO_VERSION}.linux-$(dpkg --print-architecture).tar.gz" | tar -C /usr/local -xz
ENV SEED=/opt/xherd/seed
RUN git clone -q --depth 1 -b "v${NVM_VERSION}" https://github.com/nvm-sh/nvm "$SEED/.nvm" \
    && rm -rf "$SEED/.nvm/.git" \
    && curl -fsSL "https://get.sdkman.io?rcupdate=false" | SDKMAN_DIR="$SEED/.sdkman" bash \
    && sed -i 's/^sdkman_auto_answer=.*/sdkman_auto_answer=true/' "$SEED/.sdkman/etc/config" \
    && curl -fsSL https://sh.rustup.rs \
       | RUSTUP_HOME="$SEED/.rustup" CARGO_HOME="$SEED/.cargo" \
         sh -s -- -y -q --no-modify-path --profile minimal --default-toolchain none

# Pre-seeded host keys (R-683), checked against the published fingerprints.
COPY deploy/docker/known_hosts deploy/docker/known_hosts.sha256 /opt/xherd/
RUN diff <(ssh-keygen -lf /opt/xherd/known_hosts | awk '{print $2, $3}') /opt/xherd/known_hosts.sha256

COPY --from=build /usr/local/bin/xherd /usr/local/bin/xherd

ENV LANG=C.UTF-8 \
    XHERD_SANDBOX=1 \
    XHERD_HEADLESS=1 \
    XHERD_BIND=0.0.0.0 \
    PATH=/home/xherd/.cargo/bin:/home/xherd/.local/bin:/usr/local/go/bin:/home/xherd/go/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

EXPOSE 4740
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s \
    CMD curl -fsS http://127.0.0.1:4740/api/v1/healthz || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/xherd", "init"]
