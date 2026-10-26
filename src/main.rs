use std::{
    env, fs,
    io::Write,
    process::{Command, Stdio},
};

const OUTSIDE: &str = "pifzf only works inside tmux";

#[derive(Debug, PartialEq, Eq)]
struct Pane {
    id: String,
    session: String,
    location: String,
    tty: String,
    metadata: String,
}

fn tmux(socket: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new("tmux")
        .args(["-S", socket])
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

fn panes(output: &str) -> Vec<Pane> {
    output
        .lines()
        .filter_map(|line| {
            let mut f = line.splitn(5, '\t');
            Some(Pane {
                id: f.next()?.to_owned(),
                session: f.next()?.to_owned(),
                location: f.next()?.to_owned(),
                tty: f.next()?.to_owned(),
                metadata: f.next()?.to_owned(),
            })
        })
        .collect()
}

// /proc/<pid>/stat: comm can contain spaces and parentheses; starttime is field 22.
fn starttime(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    Some(
        stat.rsplit_once(") ")?
            .1
            .split_whitespace()
            .nth(19)?
            .to_owned(),
    )
}

fn launch_cwd(hex: &str) -> Option<String> {
    if hex.len() % 2 != 0 || hex.is_empty() {
        return None;
    }
    let bytes = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok().filter(|s| s.starts_with('/'))
}

fn valid(p: &Pane) -> Option<(&'static str, String)> {
    let mut fields = p.metadata.split(':');
    let pid: u32 = fields.next()?.parse().ok()?;
    let started = fields.next()?;
    let tty = fields.next()?;
    let status = fields.next()?;
    let cwd = launch_cwd(fields.next()?)?;
    if fields.next().is_some()
        || !matches!(
            status,
            "blocking" | "running" | "subagents-running" | "idle"
        )
        || !p.tty.starts_with("/dev/pts/")
        || p.tty != tty
        || starttime(pid)?.as_str() != started
        || fs::read_link(format!("/proc/{pid}/fd/0")).ok()?.to_str()? != tty
    {
        return None;
    }
    Some((
        match status {
            "blocking" => "blocking",
            "running" => "running",
            "subagents-running" => "subagents-running",
            _ => "idle",
        },
        cwd,
    ))
}

fn choose<'a>(panes: &'a [Pane], id: &str) -> Option<(&'a Pane, &'static str, String)> {
    panes.iter().find(|p| p.id == id).and_then(|p| {
        let (s, cwd) = valid(p)?;
        Some((p, s, cwd))
    })
}

fn list(socket: &str) -> Result<Vec<Pane>, String> {
    Ok(panes(&tmux(socket, &["list-panes", "-a", "-F", "#{pane_id}\t#{session_id}\t#{session_name}:#{window_index}.#{pane_index}\t#{pane_tty}\t#{@pifzf}"])?))
}

fn run() -> Result<(), String> {
    let socket = env::var("TMUX")
        .ok()
        .and_then(|value| {
            let (server, session) = value.rsplit_once(',')?;
            let (socket, pid) = server.rsplit_once(',')?;
            if !socket.starts_with('/')
                || pid.parse::<u32>().is_err()
                || session.parse::<u32>().is_err()
            {
                return None;
            }
            Some(socket.to_owned())
        })
        .ok_or(OUTSIDE)?;
    let pane = env::var("TMUX_PANE").unwrap_or_default();
    let tty = fs::read_link("/proc/self/fd/0").ok();
    let registered_tty = tmux(
        &socket,
        &["display-message", "-p", "-t", &pane, "#{pane_tty}"],
    )
    .ok();
    if !pane.starts_with('%')
        || !matches!((tty.as_ref().and_then(|t| t.to_str()), registered_tty.as_deref()),
            (Some(a), Some(b)) if a == b.trim() && a.starts_with("/dev/pts/"))
    {
        return Err(OUTSIDE.into());
    }
    let entries: Vec<_> = list(&socket)?
        .into_iter()
        .filter_map(|p| valid(&p).map(|(s, cwd)| (p, s, cwd)))
        .collect();
    if entries.is_empty() {
        println!("No interactive Pi sessions found in tmux.");
        return Ok(());
    }
    let mut fzf = Command::new("fzf")
        .args(["--ansi", "--delimiter=\t", "--with-nth=2..", "--no-multi"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    {
        let mut stdin = fzf.stdin.take().ok_or("fzf: stdin unavailable")?;
        for (p, status, cwd) in entries {
            let (color, label) = match status {
                "blocking" => ("31", "blocking"),
                "running" => ("33", "running"),
                "subagents-running" => ("35", "subagents-running"),
                _ => ("32", "idle"),
            };
            // No untrusted control sequences from cwd/session names in the selector.
            let clean = |s: &str| {
                s.chars()
                    .filter(|c| !c.is_control() && *c != '\u{1b}')
                    .collect::<String>()
            };
            writeln!(
                stdin,
                "{}\t\x1b[{color}m{label}\x1b[0m  {}  {}",
                p.id,
                clean(&p.location),
                clean(&cwd)
            )
            .map_err(|e| e.to_string())?;
        }
    }
    let output = fzf.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        if output.status.code() == Some(1) || output.status.code() == Some(130) {
            return Ok(());
        }
        return Err(format!("fzf: {}", output.status));
    }
    let selected = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let id = selected.split('\t').next().unwrap_or("");
    let current = list(&socket)?;
    let (pane, _, _) = choose(&current, id).ok_or("The selected Pi pane no longer exists.")?;
    tmux(&socket, &["switch-client", "-t", &pane.session])?;
    tmux(&socket, &["select-window", "-t", &pane.id])?;
    tmux(&socket, &["select-pane", "-t", &pane.id])?;
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_uses_exact_id_and_status_validation() {
        let pid = std::process::id();
        let tty = fs::read_link(format!("/proc/{pid}/fd/0")).unwrap();
        let tty = tty.to_str().unwrap();
        let start = starttime(pid).unwrap();
        let raw = format!("%12\t$1\ta:0.1\t{tty}\t{pid}:{start}:{tty}:running:2f70726f6a656374\n%123\t$1\ta:0.2\t{tty}\t{pid}:0:{tty}:idle:2f746d70");
        let ps = panes(&raw);
        assert_eq!(
            choose(&ps, "%12").map(|(p, _, cwd)| (p.id.as_str(), cwd)),
            if tty.starts_with("/dev/pts/") {
                Some(("%12", "/project".to_owned()))
            } else {
                None
            }
        );
        assert_eq!(ps.iter().find(|p| p.id == "%12").unwrap().location, "a:0.1");
        assert_eq!(
            ps.iter().find(|p| p.id == "%123").unwrap().location,
            "a:0.2"
        );
        assert!(choose(&ps, "%123").is_none());
        assert!(choose(&ps, "%1").is_none());
        assert_eq!(launch_cwd("2f70726f6a656374"), Some("/project".into()));
        assert_eq!(launch_cwd("2f746d700a"), Some("/tmp\n".into()));
        assert!(launch_cwd("2fgg").is_none());
    }
}
