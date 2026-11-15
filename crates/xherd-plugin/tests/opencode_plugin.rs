//! The opencode plugin xherd writes, run under Node with a fake opencode
//! client (#145): a slash command that reaches the model as plain text (the
//! TUI submitted `--prompt` before its command list loaded) is expanded by
//! the plugin into the command's template.

use std::path::Path;
use std::process::Command;

use serde_json::Value;
use xherd_plugin::runtime::{Allow, Terminal, hook_file_text};

/// Imports the rendered plugin, plays each case on a fresh plugin instance
/// and prints `{case: text after the hooks}` as JSON.
const HARNESS: &str = r##"
import { XherdPlugin } from "./plugin.mjs";

const COMMANDS = [
  { name: "xherd/feature", template: "# Feature\nTicket: $ARGUMENTS" },
  { name: "xherd/next", template: "# Implement the next task" },
];
const listing = { list: async () => ({ data: COMMANDS }) };
const broken = { list: async () => { throw new Error("server down"); } };
const throwing = { list: () => { throw new Error("no client"); } };

async function run(command, text, viaCommand) {
  const plugin = await XherdPlugin({ client: { command, session: {} } });
  const sessionID = "ses_test";
  if (viaCommand) {
    const m = text.match(/^\/(\S+)\s*([\s\S]*)$/);
    await plugin["command.execute.before"]({ sessionID, command: m[1], arguments: m[2] }, {});
  }
  const output = { message: { role: "user" }, parts: [{ type: "text", text }] };
  const done = plugin["chat.message"]({ sessionID }, output).then(() => "resolved", (e) => "rejected: " + e);
  const timeout = new Promise((r) => setTimeout(() => r("timeout"), 5000));
  const hook = await Promise.race([done, timeout]);
  return { hook, text: output.parts.map((p) => p.text).join("\n") };
}

const out = {
  feature: await run(listing, "/xherd/feature https://x.test/issues/1 ", false),
  append: await run(listing, "/xherd/next T05", false),
  plain: await run(listing, "Please fix the build", false),
  unknown: await run(listing, "/nope x", false),
  viaCommand: await run(listing, "/xherd/feature https://x.test/issues/1", true),
  listRejects: await run(broken, "/xherd/feature https://x.test/issues/1 ", false),
  listThrows: await run(throwing, "/xherd/feature https://x.test/issues/1 ", false),
};
console.log(JSON.stringify(out));
process.exit(0);
"##;

fn node() -> Option<String> {
    let ok = Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    ok.then(|| "node".to_string())
}

fn render(dir: &Path) {
    let m = xherd_plugin::builtins()
        .into_iter()
        .find(|m| m.name == "opencode")
        .unwrap();
    let hooks = Terminal::of(&m).unwrap().hooks.unwrap();
    // `/bin/true`: the plugin's `xherd hook` calls do nothing.
    let js = hook_file_text(None, &hooks, "/bin/true", &Allow::default()).unwrap();
    std::fs::write(dir.join("plugin.mjs"), js).unwrap();
    std::fs::write(dir.join("harness.mjs"), HARNESS).unwrap();
}

#[test]
fn a_command_sent_as_plain_text_is_expanded() {
    let Some(node) = node() else {
        eprintln!("skipped: node is not on PATH");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    render(dir.path());
    let out = Command::new(node)
        .arg("harness.mjs")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "harness failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let r: Value = serde_json::from_slice(&out.stdout).unwrap();
    let text = |case: &str| r[case]["text"].as_str().unwrap().to_string();
    for case in [
        "feature",
        "append",
        "plain",
        "unknown",
        "viaCommand",
        "listRejects",
        "listThrows",
    ] {
        assert_eq!(r[case]["hook"], "resolved", "{case}: {r}");
    }

    // R1: rendered like opencode renders the command.
    assert_eq!(
        text("feature"),
        "# Feature\nTicket: https://x.test/issues/1"
    );
    let append = text("append");
    assert!(
        append.starts_with("# Implement the next task\n") && append.trim_end().ends_with("\nT05"),
        "arguments appended to a template without placeholders: {append:?}"
    );

    // R2: left as they are.
    assert_eq!(text("plain"), "Please fix the build");
    assert_eq!(text("unknown"), "/nope x");
    assert_eq!(text("viaCommand"), "/xherd/feature https://x.test/issues/1");

    // R3: no command list, no change.
    assert_eq!(
        text("listRejects"),
        "/xherd/feature https://x.test/issues/1 "
    );
    assert_eq!(
        text("listThrows"),
        "/xherd/feature https://x.test/issues/1 "
    );
}
