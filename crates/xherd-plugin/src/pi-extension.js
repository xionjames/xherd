// Written by xherd for this agent's worktree and replaced when the agent is
// prepared again; do not edit (R-005b). It reports the session to xherd with
// `xherd hook <event>` and, by permission mode, asks before tools run. # xherd
import { spawn } from "node:child_process";
import { resolve, sep } from "node:path";

const XHERD = __XHERD__;
// Pi event -> xherd hook event.
const EVENTS = __EVENTS__;
// Shell commands (a trailing `*` matches the rest) and folders xherd's
// commands use without asking.
const COMMANDS = __COMMANDS__;
const READ_DIRS = __READ_DIRS__;
// ask | edits | auto | all (the launch's permission mode).
const MODE = process.env.XHERD_PI_MODE || "ask";

// One call at a time, in order: xherd numbers the reports as they arrive.
let chain = Promise.resolve();
function hook(trigger, payload) {
  const event = EVENTS[trigger];
  if (!event) return chain;
  chain = chain.then(
    () =>
      new Promise((done) => {
        try {
          const p = spawn(XHERD, ["hook", event], { stdio: ["pipe", "ignore", "ignore"] });
          p.on("error", () => done());
          p.on("exit", () => done());
          p.stdin.on("error", () => {});
          p.stdin.end(JSON.stringify(payload));
        } catch {
          done();
        }
      }),
  );
  return chain;
}

const allowed = (command) =>
  COMMANDS.some((c) => (c.endsWith("*") ? command.startsWith(c.slice(0, -1)) : command === c));

const inside = (path, dir) => path === dir || path.startsWith(dir.endsWith(sep) ? dir : dir + sep);

// What the mode asks about. Pi has no sandbox: this gate is a convenience
// for `ask`, `edits` and `auto`, not a security boundary.
function question(event, cwd) {
  if (MODE === "all") return undefined;
  const input = event.input ?? {};
  if (typeof input.path === "string") {
    const path = resolve(cwd, input.path);
    const known = inside(path, cwd) || READ_DIRS.some((d) => inside(path, d));
    const reads = ["read", "grep", "find", "ls"].includes(event.toolName);
    if (!known) return `${event.toolName} outside the worktree: ${path}`;
    if (MODE === "ask" && !reads) return `${event.toolName}: ${path}`;
  }
  if (event.toolName === "bash" && MODE !== "auto") {
    const command = String(input.command ?? "").trim();
    if (!allowed(command)) return `bash: ${command}`;
  }
  return undefined;
}

const textOf = (message) =>
  (Array.isArray(message?.content) ? message.content : [])
    .filter((b) => b?.type === "text")
    .map((b) => b.text)
    .join("\n");

export default function (pi) {
  // The prompt of the run being started, reported once: `input` carries the
  // raw text (`/xherd-next …`), `before_agent_start` covers a first prompt
  // that came on the launch line.
  let reported = false;
  const session = (ctx) => {
    try {
      return ctx.sessionManager.getSessionId();
    } catch {
      return undefined;
    }
  };

  pi.on("session_start", async (_event, ctx) => {
    await hook("session_start", { session_id: session(ctx) });
  });
  pi.on("input", async (event, ctx) => {
    if (event.source === "extension") return;
    reported = true;
    await hook("input", { session_id: session(ctx), prompt: event.text });
  });
  pi.on("before_agent_start", async (event, ctx) => {
    if (!reported && event.messageType === "user") {
      await hook("before_agent_start", { session_id: session(ctx), prompt: event.prompt ?? "" });
    }
    reported = false;
  });
  pi.on("tool_call", async (event, ctx) => {
    const ask = question(event, ctx.cwd);
    if (!ask) return;
    if (!ctx.hasUI) return { block: true, reason: `Not allowed without asking: ${ask}` };
    const ok = await ctx.ui.confirm("xherd: allow?", ask);
    if (!ok) return { block: true, reason: "Denied by the user" };
  });
  pi.on("tool_execution_end", async (event, ctx) => {
    await hook("tool_execution_end", { session_id: session(ctx), tool_name: event.toolName });
  });
  pi.on("ui_prompt_start", async (event, ctx) => {
    await hook("ui_prompt_start", {
      session_id: session(ctx),
      message: event.title ? `${event.kind}: ${event.title}` : String(event.kind ?? "a question"),
    });
  });
  pi.on("agent_settled", async (_event, ctx) => {
    let message = "";
    try {
      const last = [...ctx.sessionManager.getBranch()]
        .reverse()
        .find((e) => e.type === "message" && e.message?.role === "assistant");
      message = textOf(last?.message);
    } catch {}
    await hook("agent_settled", { session_id: session(ctx), message });
  });
}
