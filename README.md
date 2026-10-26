# pifzf

A selector for interactive Pi instances in panes on the current tmux server (Linux). No daemon or screen scraping.

## Installation

```sh
nix profile install github:moreauadrien/pifzf#pifzf
pi install git:github.com/moreauadrien/pifzf
```

Until then, from the repository root:

```sh
nix profile install path:.#pifzf
pi install "$PWD"
```

The local Pi package refers to this directory; keep it in place. Restart Pi in a tmux pane (or run `/reload` after extension changes), then run `pifzf` from a tmux client. The Nix package includes `tmux` and `fzf` in the CLI's runtime `PATH`; Pi still needs `tmux` on its own `PATH`. You can try the CLI without installing it with `nix run path:.#pifzf`.

Optional: `pi install npm:@gotgenes/pi-subagents` enables the `subagents-running` state; `pi install npm:@juicesharp/rpiv-ask-user-question` adds interactive questions. The latter uses Pi UI prompts, so an open question produces the `blocking` state (as do other `select`, `confirm`, `input`, `editor`, and `custom` UI prompts). Neither plugin is required for basic operation.

## States

The selector refreshes its list every 500 ms while open, reloading fzf only when a pane's displayed state or location changes. If all Pi panes disappear, the list becomes empty until a new session appears.

Priority: `blocking` > `running` (parent agent, from `before_agent_start` until `agent_settled`) > `subagents-running` (actual `subagents:started/resuming` events until `completed/failed/resumed`; queued agents do not count) > `idle`. Only TUI Pi processes whose stdin is attached to their pane's tty register themselves. A Pi started before the extension loaded, or an extension reloaded while a subagent is already running, may miss that subagent until its next event; other subagent plugins are not tracked. TUI prompts that do not use Pi's UI hooks cannot be detected. After `SIGKILL`, tmux metadata may remain: the CLI rejects dead or reused PIDs, a changed process start time, or a mismatched tty. The extension captures `process.cwd()` when loaded and publishes that launch directory (hex-encoded in a tmux option); the CLI displays it rather than `pane_current_path`. Both the extension and CLI target the socket specified by `TMUX` and verify that `TMUX_PANE` belongs to that server and matches their tty.

## Tests

```sh
cargo test
node tests/status.mjs
```

Tests cover launch-path decoding, pane validation, selection by ID, state priority, and execution outside tmux. Selection uses the immutable `%pane` ID and revalidates it before `switch-client` / `select-window` / `select-pane` (including across sessions). No Pi found: print a message without launching fzf. Pane removed after selection: report an error. Outside tmux: `pifzf only works inside tmux`.
