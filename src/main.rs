use std::{
    env, fs,
    io::Write,
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
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

fn display(socket: &str) -> Result<String, String> {
    let mut result = String::new();
    for p in list(socket)? {
        let Some((status, cwd)) = valid(&p) else { continue };
        result.push_str(&format_row(&p, status, &cwd));
    }
    Ok(result)
}

fn format_row(p: &Pane, status: &str, cwd: &str) -> String {
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
    format!(
        "{}\t\x1b[{color}m{label}\x1b[0m  {}  {}\n",
        p.id,
        clean(&p.location),
        clean(cwd)
    )
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
    let initial = display(&socket)?;
    if initial.is_empty() {
        println!("No interactive Pi sessions found in tmux.");
        return Ok(());
    }
    // fzf's HTTP listener accepts reload actions; polling tmux only reloads when the list changes.
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let quoted = exe.to_string_lossy().replace('\'', "'\\''");
    let action = format!("reload('{quoted}' --list)");
    let mut fzf = Command::new("fzf")
        .args([
            "--ansi",
            "--delimiter=\t",
            "--with-nth=2..",
            "--no-multi",
            &format!("--listen={port}"),
        ])
        .env("PIFZF_SOCKET", &socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    fzf.stdin
        .take()
        .ok_or("fzf: stdin unavailable")?
        .write_all(initial.as_bytes())
        .map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let watch_socket = socket.clone();
    let watcher = thread::spawn(move || {
        let mut previous = initial;
        while !stopped.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(500));
            if stopped.load(Ordering::Relaxed) {
                break;
            }
            if let Ok(next) = display(&watch_socket) {
                if next != previous {
                    if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
                        let request = format!("POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{action}", action.len());
                        if stream.write_all(request.as_bytes()).is_ok() {
                            previous = next;
                        }
                    }
                }
            }
        }
    });
    let output = fzf.wait_with_output().map_err(|e| e.to_string());
    stop.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    let output = output?;
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
    if env::args().nth(1).as_deref() == Some("--list") {
        let result = env::var("PIFZF_SOCKET")
            .map_err(|e| e.to_string())
            .and_then(|socket| display(&socket));
        match result {
            Ok(rows) => print!("{rows}"),
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn row_reflects_state_changes() {
        let pane = Pane {
            id: "%12".into(),
            session: "$1".into(),
            location: "main:0.1".into(),
            tty: "/dev/pts/1".into(),
            metadata: String::new(),
        };
        assert_ne!(
            format_row(&pane, "idle", "/tmp"),
            format_row(&pane, "running", "/tmp")
        );
        assert!(format_row(&pane, "blocking", "/tmp").contains("\x1b[31mblocking"));
        assert_eq!(
            format_row(&pane, "idle", "/tmp\nmalicious")
                .matches('\n')
                .count(),
            1
        );
    }

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
