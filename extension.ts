import { readFileSync, readlinkSync } from "node:fs";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// Metadata belongs to the pane, not the session; the CLI also checks /proc for stale entries.
function starttime(): string {
  return readFileSync(`/proc/${process.pid}/stat`, "utf8").split(/\) /).at(-1)!.split(" ")[19];
}

export function status(blocking: number, running: boolean, children: boolean) {
  return blocking > 0 ? "blocking" : running ? "running" : children ? "subagents-running" : "idle";
}

export default function (pi: ExtensionAPI) {
  let pane: string | undefined;
  let tty: string | undefined;
  let socket: string | undefined;
  const launchCwd = Buffer.from(process.cwd()).toString("hex");
  let running = false;
  let blocking = 0;
  let active = false;
  let last = "";
  const children = new Set<string>();
  const unsubscribe: Array<() => void> = [];

  let pending = Promise.resolve();
  const command = async (args: string[]) => {
    if (!socket) return undefined;
    const result = await pi.exec("tmux", ["-S", socket, ...args]);
    return result.code === 0 ? result.stdout.trim() : undefined;
  };
  const publish = () => {
    pending = pending.then(async () => {
      if (!active || !pane || !tty) return;
      const value = `${process.pid}:${starttime()}:${tty}:${status(blocking, running, children.size > 0)}:${launchCwd}`;
      if (value !== last && await command(["set-option", "-p", "-t", pane, "@pifzf", value]) !== undefined) last = value;
    }).catch(() => {});
    return pending;
  };
  const clear = async () => {
    active = false;
    await pending;
    if (pane) {
      // Only remove our own value: an older runtime may shut down after a replacement published.
      if (last && await command(["show-option", "-p", "-v", "-t", pane, "@pifzf"]) === last) {
        await command(["set-option", "-p", "-u", "-t", pane, "@pifzf"]);
      }
    }
    last = "";
    pane = undefined;
    tty = undefined;
    socket = undefined;
    running = false;
    blocking = 0;
    children.clear();
  };

  pi.on("session_start", async (_event, ctx) => {
    await clear();
    if (ctx.mode !== "tui" || !process.stdin.isTTY || !/^%\d+$/.test(process.env.TMUX_PANE ?? "")) return;
    // TMUX contains <socket>,<server-pid>,<session-id>; use that socket explicitly,
    // never tmux's default server if the environment is stale or malformed.
    const match = /^(\/.*),(\d+),(\d+)$/.exec(process.env.TMUX ?? "");
    if (!match) return;
    socket = match[1];
    const target = process.env.TMUX_PANE!;
    let inputTty: string;
    try { inputTty = readlinkSync("/proc/self/fd/0"); } catch { return; }
    if (!inputTty.startsWith("/dev/pts/")) return;
    // Explicit TMUX socket: verify this pane exists on that server and owns our tty.
    if (await command(["display-message", "-p", "-t", target, "#{pane_id} #{pane_tty}"]) !== `${target} ${inputTty}`) return;
    pane = target;
    tty = inputTty;
    active = true;
    subscribe();
    await publish();
  });
  pi.on("before_agent_start", async () => { running = true; await publish(); });
  pi.on("agent_settled", async () => { running = false; await publish(); });
  pi.on("ui_prompt_start", async () => { blocking++; await publish(); });
  pi.on("ui_prompt_end", async () => { blocking = Math.max(0, blocking - 1); await publish(); });
  function subscribe() {
    if (unsubscribe.length) return;
    for (const event of ["subagents:started", "subagents:resuming"] as const) {
      unsubscribe.push(pi.events.on(event, (data: unknown) => {
        if (active && data && typeof data === "object" && "id" in data && typeof data.id === "string") children.add(data.id);
        void publish();
      }));
    }
    for (const event of ["subagents:completed", "subagents:failed", "subagents:resumed"] as const) {
      unsubscribe.push(pi.events.on(event, (data: unknown) => {
        if (data && typeof data === "object" && "id" in data && typeof data.id === "string") children.delete(data.id);
        void publish();
      }));
    }
  }
  pi.on("session_shutdown", async () => {
    for (const off of unsubscribe.splice(0)) off();
    await clear();
  });
}
