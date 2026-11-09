//! Names and paths of the xherd container (design §8.4, R-1203): the daemon
//! and the agents run as different users there. The image, `xherd init` and
//! the CLI all use these.

/// The daemon's own user; its data is unreadable by the agents' user.
pub const DAEMON_USER: &str = "xherd-daemon";
/// The daemon's home, on its own volume. `data` is 0700; `shared` is
/// readable by everyone (files the daemon writes for the agents' user).
pub const DAEMON_HOME: &str = "/var/lib/xherd-daemon";
pub const DAEMON_DATA: &str = "/var/lib/xherd-daemon/data";
pub const SHARED: &str = "/var/lib/xherd-daemon/shared";
/// The agents' user's home (the host-mapped user; the `xherd-home` volume).
pub const AGENT_HOME: &str = "/home/xherd";
/// Run by the agents' user: tmux server and runner sockets, shared with the
/// daemon through the host's group.
pub const RUN_DIR: &str = "/run/xherd";
pub const RUNNER_SOCK: &str = "/run/xherd/runner.sock";
pub const TMUX_SOCK: &str = "/run/xherd/tmux.sock";
/// Hooks that cannot reach the daemon wait here (agents append, the daemon
/// replays): a folder both can write, not the daemon's private data.
pub const QUEUE: &str = "/run/xherd/hook-queue.jsonl";
