//! Real CLI rejection tests. No authentication or privileged command is executed.
use std::process::Command;

fn wrapper() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sudo-pop"));
    cmd.env_remove("SUDO_POP_MODE")
        .env_remove("SUDO_POP_RUN0")
        .env_remove("SUDO_POP_TEST_EXEC")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env("PATH", "/nonexistent/sudo-pop-test");
    cmd
}

#[test]
fn unsupported_requests_fail_without_sudo_or_a_display() {
    for args in [
        vec![],
        vec!["--"],
        vec!["-v"],
        vec!["-k"],
        vec!["-l"],
        vec!["-E", "true"],
        vec!["-u", "root", "true"],
        vec!["-A", "true"],
        vec!["-n", "true"],
        vec!["-S", "true"],
        vec!["VAR=1", "true"],
    ] {
        let out = wrapper().args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("run0-only mode"));
        assert!(out.stdout.is_empty());
    }
}

#[test]
fn invalid_policy_and_legacy_override_are_errors() {
    for (name, value, expected) in [
        ("SUDO_POP_MODE", "typo", "SUDO_POP_MODE must be"),
        ("SUDO_POP_RUN0", "0", "conflicts with run0-only mode"),
    ] {
        let out = wrapper().env(name, value).arg("true").output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&out.stderr).contains(expected));
    }
}

#[test]
fn help_describes_default_and_explicit_escape_route() {
    let out = wrapper().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("run0 only"));
    assert!(text.contains("/usr/bin/sudo"));
}
