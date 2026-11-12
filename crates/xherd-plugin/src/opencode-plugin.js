// Written by xherd for this agent's worktree and replaced when the agent is
// prepared again; do not edit (R-005b). It reports the session to xherd with
// `xherd hook <event>` and allows the calls xherd's commands make. # xherd
import { spawn } from "node:child_process";

const XHERD = __XHERD__;
// opencode trigger -> xherd hook event.
const EVENTS = __EVENTS__;
// Shell commands (opencode bash patterns) and folders xherd's commands read.
const COMMANDS = __COMMANDS__;
const READ_DIRS = __READ_DIRS__;

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

// A rule set may be one action ("ask") or a map of patterns.
function rules(perm, key) {
  const v = perm[key];
  if (v && typeof v === "object") return v;
  return (perm[key] = v === undefined ? {} : { "*": v });
}

export const XherdPlugin = async ({ client }) => {
  // The slash command a prompt came from, per session.
  const commands = new Map();
  // Subagent sessions: their turns are part of the agent's, never a stop.
  const children = new Set();
  const mine = (id) => id && !children.has(id);
  // Permission prompts on screen: a tool that ends meanwhile (run in
  // parallel) must not report the agent working again.
  const asking = new Set();
  hook("start", {});
  // A continued session (`--session`) ignores `--prompt`: xherd passes its
  // first prompt here and the plugin runs it in that session, once.
  const prompt = process.env.XHERD_PROMPT;
  const session = process.env.XHERD_PROMPT_SESSION;
  if (prompt && session) {
    setTimeout(() => {
      const slash = prompt.match(/^\/(\S+)\s*([\s\S]*)$/);
      const path = { id: session };
      (slash
        ? client.session.command({ path, body: { command: slash[1], arguments: slash[2] } })
        : client.session.prompt({ path, body: { parts: [{ type: "text", text: prompt }] } })
      ).catch(() => {});
    }, 0);
  }
  return {
    config: async (cfg) => {
      const perm = (cfg.permission ??= {});
      if (COMMANDS.length) {
        const bash = rules(perm, "bash");
        for (const c of COMMANDS) bash[c] = "allow";
      }
      if (READ_DIRS.length) {
        const ext = rules(perm, "external_directory");
        for (const d of READ_DIRS) ext[d + "/*"] = "allow";
      }
    },
    "command.execute.before": async (input) => {
      commands.set(input.sessionID, `/${input.command} ${input.arguments ?? ""}`.trim());
    },
    "chat.message": async (input, output) => {
      if (!mine(input.sessionID)) return;
      const text = (output.parts ?? [])
        .filter((p) => p.type === "text")
        .map((p) => p.text)
        .join("\n");
      const prompt = commands.get(input.sessionID) ?? text;
      commands.delete(input.sessionID);
      await hook("chat.message", { session_id: input.sessionID, prompt });
    },
    "tool.execute.after": async (input) => {
      if (!mine(input.sessionID) || asking.size) return;
      await hook("tool.execute.after", { session_id: input.sessionID, tool_name: input.tool });
    },
    event: async ({ event }) => {
      const props = event.properties ?? {};
      if (event.type === "session.created" && props.info?.parentID) {
        children.add(props.info.id);
        return;
      }
      switch (event.type) {
        case "permission.replied":
        case "question.replied":
        case "question.rejected":
          asking.delete(props.requestID ?? props.id);
          break;
        case "permission.asked":
        case "question.asked":
          asking.add(props.id);
          // A subagent's prompt still waits for the user: report it, under
          // no session id (the agent's own session stays the one resumed).
          await hook(event.type, {
            session_id: mine(props.sessionID) ? props.sessionID : undefined,
            message: props.permission ? `permission: ${props.permission}` : "a question",
          });
          break;
        case "session.idle": {
          if (!mine(props.sessionID)) break;
          let message = "";
          try {
            const r = await client.session.messages({ path: { id: props.sessionID } });
            const last = [...(r.data ?? [])].reverse().find((m) => m.info?.role === "assistant");
            message = (last?.parts ?? [])
              .filter((p) => p.type === "text")
              .map((p) => p.text)
              .join("\n");
          } catch {}
          await hook("session.idle", { session_id: props.sessionID, message });
          break;
        }
      }
    },
  };
};
