use std::process::Command;

#[test]
fn outside_tmux() {
    let output = Command::new(env!("CARGO_BIN_EXE_pifzf"))
        .env_remove("TMUX")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "pifzf only works inside tmux\n"
    );
    let wrong_socket = Command::new(env!("CARGO_BIN_EXE_pifzf"))
        .env("TMUX", "/tmp/pifzf-nonexistent-socket,123,0")
        .env("TMUX_PANE", "%1")
        .output()
        .unwrap();
    assert!(!wrong_socket.status.success());
    assert_eq!(wrong_socket.stderr, output.stderr);
}
