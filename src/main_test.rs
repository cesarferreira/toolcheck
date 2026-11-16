#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

// Limit shell startup contention so short-deadline fixtures remain meaningful.
static FIXTURE_LOCK: Mutex<()> = Mutex::new(());
use tempfile::{TempDir, tempdir};

struct Fixture {
    _guard: MutexGuard<'static, ()>,
    root: TempDir,
    bins: Vec<PathBuf>,
}

impl Fixture {
    fn new() -> Self {
        let guard = FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = tempdir().unwrap();
        let bins = vec![root.path().join("first"), root.path().join("second")];
        for bin in &bins {
            fs::create_dir(bin).unwrap();
        }
        Self {
            root,
            bins,
            _guard: guard,
        }
    }

    fn tool(&self, bin: usize, name: &str, body: &str) {
        let path = self.bins[bin].join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn policy(&self, tools: &str) {
        fs::write(
            self.root.path().join("toolcheck.toml"),
            format!("schema_version = 1\n{tools}\n"),
        )
        .unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_toolcheck"))
            .args(args)
            .current_dir(self.root.path())
            .env_clear()
            .env("PATH", std::env::join_paths(&self.bins).unwrap())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                panic!("toolcheck exceeded fixture watchdog");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        child.wait_with_output().unwrap()
    }

    fn json(&self, extra: &[&str], code: i32) -> Value {
        let mut args = vec!["--json"];
        args.extend(extra);
        let output = self.run(&args);
        assert_eq!(
            output.status.code(),
            Some(code),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice(&output.stdout).expect("stdout must be a single JSON value")
    }
}

#[test]
fn numeric_ordering_and_human_guidance() {
    let f = Fixture::new();
    f.tool(0, "dekit", "echo dekit v0.10.10");
    f.policy("[tools.dekit]\nminimum = '0.10.3'\nreason = 'Graphics support'\nupgrade = 'never-run-this'");
    let json = f.json(&[], 0);
    let t = &json["tools"][0];
    assert_eq!(t["status"], "pass");
    assert_eq!(t["actual"], "0.10.10");
    assert_eq!(t["required"], ">=0.10.3");
    assert_eq!(
        t["selected_path"],
        f.bins[0].join("dekit").to_str().unwrap()
    );
    assert_eq!(t["reason"], "Graphics support");
    assert_eq!(t["remediation"], "never-run-this");
    let output = f.run(&["check"]);
    let human = String::from_utf8(output.stdout).unwrap();
    for text in [
        "PASS",
        "0.10.10",
        ">=0.10.3",
        "Graphics support",
        "never-run-this",
    ] {
        assert!(human.contains(text), "{human}");
    }
    assert!(human.contains(f.bins[0].to_str().unwrap()));
}

#[test]
fn exact_file_is_relative_to_manifest() {
    let f = Fixture::new();
    f.tool(0, "ruby", "echo ruby 3.3");
    let policy_dir = f.root.path().join("policy");
    fs::create_dir(&policy_dir).unwrap();
    fs::write(policy_dir.join(".ruby-version"), "v3.3.0\n").unwrap();
    fs::write(
        policy_dir.join("tools.toml"),
        "schema_version=1\n[tools.ruby]\nexact_from='.ruby-version'\n",
    )
    .unwrap();
    let json = f.json(&["check", "--config", "policy/tools.toml"], 0);
    assert_eq!(json["tools"][0]["actual"], "3.3.0");
    assert_eq!(json["tools"][0]["required"], "=3.3.0");
}

#[test]
fn exact_and_maximum_are_enforced() {
    let f = Fixture::new();
    f.tool(0, "a", "echo 2.1");
    for constraint in ["exact='2.0'", "maximum='2.0'"] {
        f.policy(&format!("[tools.a]\n{constraint}"));
        assert_eq!(f.json(&[], 1)["tools"][0]["status"], "error");
    }
}

#[test]
fn regex_stderr_and_literal_arguments() {
    let f = Fixture::new();
    f.tool(0, "aa", "[ \"$1\" = '$(touch owned)' ] || exit 8\necho 'build=release_1_4_2 version: v1.4.2 other: 9.9.9' >&2");
    f.policy("[tools.opendhu]\ncommand='aa'\nminimum='1.4'\nversion_args=['$(touch owned)']\nversion_regex='version: (v[0-9.]+)'");
    assert_eq!(f.json(&[], 0)["tools"][0]["actual"], "1.4.2");
    assert!(!f.root.path().join("owned").exists());
}

#[test]
fn prerelease_requires_explicit_prerelease_bound() {
    let f = Fixture::new();
    f.tool(0, "a", "echo 2.0.0-beta.2");
    for (constraint, code) in [
        ("minimum='1.0'", 1),
        ("minimum='2.0.0-beta.1'", 0),
        ("exact='2.0.0-beta.2'", 0),
        ("minimum='2.0.0'", 1),
    ] {
        f.policy(&format!("[tools.a]\n{constraint}"));
        f.json(&[], code);
    }
}

#[test]
fn missing_optional_is_skipped_but_installed_optional_is_checked() {
    let f = Fixture::new();
    f.policy("[tools.absent]\nminimum='1'\noptional=true");
    assert_eq!(f.json(&["--strict"], 0)["tools"][0]["status"], "skipped");
    f.tool(0, "absent", "echo 0.9");
    assert_eq!(f.json(&[], 1)["tools"][0]["status"], "error");
}

#[test]
fn required_missing_and_warnings_have_consistent_exit_codes() {
    let f = Fixture::new();
    f.policy("[tools.absent]\nminimum='1'");
    assert_eq!(f.json(&[], 1)["tools"][0]["issue"], "missing");
    f.policy("[tools.absent]\nminimum='1'\nseverity='warn'");
    assert_eq!(f.json(&[], 1)["tools"][0]["status"], "error");
    f.json(&["--strict"], 1);
    f.tool(0, "absent", "echo 0.5");
    f.json(&[], 0);
    f.json(&["--strict"], 1);
}

#[test]
fn unparseable_output_and_failures_never_pass() {
    let f = Fixture::new();
    f.policy("[tools.a]\nminimum='1'");
    for (body, issue) in [
        ("echo unknown", "unknown_version"),
        ("echo 1.0.0; exit 7", "command_failed"),
        ("echo 1.2.3 4.5.6", "unknown_version"),
        ("echo 1.02.3", "unknown_version"),
    ] {
        f.tool(0, "a", body);
        let json = f.json(&[], 1);
        assert_eq!(json["tools"][0]["issue"], issue);
        assert!(json["tools"][0]["actual"].is_null());
    }
}

#[test]
fn invalid_policy_is_configuration_error() {
    let f = Fixture::new();
    for policy in [
        "[tools.a]\nminimum='1'\nminimun='2'",
        "[tools.a]\nminimum='2'\nmaximum='1'",
        "[tools.a]\nminimum='1'\nexact='1'",
        "[tools.a]\nexact='1'\nexact_from='.pin'",
        "[tools.a]\nexact='nope'",
        "[tools.a]\nminimum='1'\nversion_regex='['",
        "[tools.a]\nminimum='1'\nversion_regex='[0-9]+'",
        "[tools.a]\nminimum='1'\ntimeout_ms=0",
        "[tools.a]\nminimum='1'\ntimeout_ms=30001",
        "[tools.a]\nminimum='1'\nseverity='fatal'",
        "[tools.a]\ncommand=''\nminimum='1'",
        "[tools.a]\ncommand='a --version'\nminimum='1'",
        "[tools.a]",
    ] {
        f.policy(policy);
        let json = f.json(&[], 2);
        assert_eq!(json["error_kind"], "configuration", "{policy}");
        assert!(json["error"].as_str().unwrap().contains('a'), "{json}");
    }
}

#[test]
fn schema_and_missing_or_invalid_pin_are_configuration_errors() {
    let f = Fixture::new();
    for contents in [
        "schema_version=2",
        "schema_version=1\nunknown=true",
        "broken [",
    ] {
        fs::write(f.root.path().join("toolcheck.toml"), contents).unwrap();
        f.json(&[], 2);
    }
    f.policy("[tools.a]\nexact_from='.pin'");
    f.json(&[], 2);
    for pin in ["not a version", "1.2.3\n2.3.4", "", "01.2.3"] {
        fs::write(f.root.path().join(".pin"), pin).unwrap();
        f.json(&[], 2);
    }
    fs::remove_file(f.root.path().join(".pin")).unwrap();
    fs::create_dir(f.root.path().join(".pin")).unwrap();
    f.json(&[], 2);
}

#[test]
fn validation_completes_before_running_any_tool() {
    let f = Fixture::new();
    f.tool(0, "a", "echo ran > marker; echo 1.0");
    f.policy("[tools.a]\nminimum='1'\n[tools.z]\nminimum='invalid'");
    f.json(&[], 2);
    assert!(!f.root.path().join("marker").exists());
}

#[test]
fn timeout_includes_descendants_holding_pipes() {
    let f = Fixture::new();
    for body in ["/bin/sleep 3", "/bin/sleep 3 &\necho 1.0.0\nexit 0"] {
        f.tool(0, "a", body);
        f.policy("[tools.a]\nminimum='1'\ntimeout_ms=500");
        let start = Instant::now();
        let json = f.json(&[], 1);
        assert_eq!(json["tools"][0]["issue"], "timeout");
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}

#[test]
fn output_limit_terminates_noisy_process() {
    let f = Fixture::new();
    f.tool(
        0,
        "a",
        "while :; do echo 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done",
    );
    f.policy("[tools.a]\nminimum='1'\ntimeout_ms=2000");
    let start = Instant::now();
    assert_eq!(f.json(&[], 1)["tools"][0]["issue"], "output_limit");
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn shadowed_compatible_installation_is_explained_and_aliases_are_deduplicated() {
    let f = Fixture::new();
    f.tool(0, "a", "echo 0.9");
    f.tool(1, "a", "echo 1.5");
    f.policy("[tools.a]\nminimum='1'");
    let json = f.json(&[], 1);
    let t = &json["tools"][0];
    assert_eq!(t["shadowed"][0]["actual"], "1.5.0");
    assert_eq!(t["shadowed"][0]["compatible"], true);
    assert!(t["message"].as_str().unwrap().contains("PATH"));
    fs::remove_file(f.bins[1].join("a")).unwrap();
    symlink(f.bins[0].join("a"), f.bins[1].join("a")).unwrap();
    assert_eq!(
        f.json(&[], 1)["tools"][0]["shadowed"],
        serde_json::json!([])
    );
    fs::remove_file(f.bins[1].join("a")).unwrap();
    fs::hard_link(f.bins[0].join("a"), f.bins[1].join("a")).unwrap();
    assert_eq!(
        f.json(&[], 1)["tools"][0]["shadowed"],
        serde_json::json!([])
    );
}

#[test]
fn shadow_probes_share_the_tool_deadline() {
    let f = Fixture::new();
    f.tool(0, "a", "echo 0.5");
    f.tool(1, "a", "/bin/sleep 3");
    f.policy("[tools.a]\nminimum='1'\ntimeout_ms=500");
    let start = Instant::now();
    let json = f.json(&[], 1);
    assert_eq!(json["tools"][0]["actual"], "0.5.0", "{json}");
    assert_eq!(json["tools"][0]["shadowed"][0]["issue"], "timeout");
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn non_executable_path_entries_are_skipped() {
    let f = Fixture::new();
    fs::write(f.bins[0].join("a"), "not executable").unwrap();
    f.tool(1, "a", "echo 1.2");
    f.policy("[tools.a]\nminimum='1'");
    let json = f.json(&[], 0);
    assert_eq!(
        json["tools"][0]["selected_path"],
        f.bins[1].join("a").to_str().unwrap()
    );
}

#[test]
fn json_order_and_summary_are_deterministic() {
    let f = Fixture::new();
    f.tool(0, "z", "echo 1.0");
    f.tool(0, "b", "echo 0.5");
    f.policy("[tools.z]\nminimum='1'\n[tools.a]\nminimum='1'\noptional=true\n[tools.b]\nminimum='1'\nseverity='warn'");
    let a = f.json(&[], 0);
    let b = f.json(&[], 0);
    assert_eq!(a, b);
    assert_eq!(a["tools"][0]["name"], "a");
    assert_eq!(
        a["summary"],
        serde_json::json!({"pass":1,"warn":1,"error":0,"skipped":1,"acceptable":true})
    );
}

#[test]
fn invocation_errors_are_exit_two_and_json_when_requested() {
    let f = Fixture::new();
    let json = f.json(&["--nonsense"], 2);
    assert_eq!(json["error_kind"], "invocation");
    assert_eq!(f.run(&["install"]).status.code(), Some(2));
    assert!(f.run(&["--help"]).status.success());
}

#[test]
#[cfg(target_os = "linux")]
fn non_utf8_paths_remain_valid_json() {
    use std::os::unix::ffi::OsStringExt;
    let mut f = Fixture::new();
    let odd = f
        .root
        .path()
        .join(std::ffi::OsString::from_vec(b"bin-\xff".to_vec()));
    fs::create_dir(&odd).unwrap();
    f.bins[0] = odd;
    f.tool(0, "a", "echo 1.2.3");
    f.policy("[tools.a]\nminimum='1'");
    let json = f.json(&[], 0);
    assert_eq!(json["tools"][0]["actual"], "1.2.3");
    assert!(json["tools"][0]["selected_path"].as_str().is_some());
}

#[test]
fn bounds_are_inclusive_and_build_metadata_does_not_change_precedence() {
    let f = Fixture::new();
    f.tool(0, "a", "echo v1.2.3+local");
    for constraint in ["minimum='1.2.3'\nmaximum='1.2.3'", "exact='1.2.3+team'"] {
        f.policy(&format!("[tools.a]\n{constraint}"));
        f.json(&[], 0);
    }
}

#[test]
fn default_args_and_short_major_versions() {
    let f = Fixture::new();
    f.tool(
        0,
        "robin",
        "[ \"$#\" = 1 ] && [ \"$1\" = --version ] || exit 9; echo v2",
    );
    f.policy("[tools.robin]\nexact='2.0.0'");
    assert_eq!(f.json(&[], 0)["tools"][0]["actual"], "2.0.0");
}

#[test]
fn unknown_warning_is_visible_and_strict_fails_it() {
    let f = Fixture::new();
    f.tool(0, "a", "echo mystery");
    f.policy("[tools.a]\nminimum='1'\nseverity='warn'");
    let json = f.json(&[], 0);
    assert_eq!(json["tools"][0]["status"], "warn");
    assert_eq!(json["tools"][0]["issue"], "unknown_version");
    f.json(&["--strict"], 1);
}

#[test]
fn duplicate_probes_are_capped_and_unprobed_paths_still_reported() {
    let mut f = Fixture::new();
    for i in 2..6 {
        let bin = f.root.path().join(format!("bin{i}"));
        fs::create_dir(&bin).unwrap();
        f.bins.push(bin);
    }
    for i in 0..6 {
        f.tool(i, "a", "echo ran >> probes; echo 0.1");
    }
    f.policy("[tools.a]\nminimum='1'\ntimeout_ms=4000");
    let json = f.json(&[], 1);
    assert_eq!(json["tools"][0]["shadowed"].as_array().unwrap().len(), 5);
    assert_eq!(
        fs::read_to_string(f.root.path().join("probes"))
            .unwrap()
            .lines()
            .count(),
        4
    );
    assert_eq!(json["tools"][0]["shadowed"][3]["issue"], "not_probed");
}

#[test]
fn combined_stdout_stderr_limit_is_enforced() {
    let f = Fixture::new();
    f.tool(0, "a", "i=0; while [ \"$i\" -lt 600 ]; do echo 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; echo 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx' >&2; i=$((i+1)); done");
    f.policy("[tools.a]\nminimum='1'");
    assert_eq!(f.json(&[], 1)["tools"][0]["issue"], "output_limit");
}

#[test]
fn ruby_style_output_ignores_dates_and_revisions() {
    let f = Fixture::new();
    f.tool(
        0,
        "ruby",
        "echo 'ruby 3.3.0 (2025-12-25 revision 123456) [arm64-darwin25]'",
    );
    f.policy("[tools.ruby]\nminimum='3.3'");
    assert_eq!(f.json(&[], 0)["tools"][0]["actual"], "3.3.0");
}

#[test]
fn non_regular_pin_is_rejected_without_blocking() {
    use std::os::unix::ffi::OsStrExt;
    let f = Fixture::new();
    let path = f.root.path().join(".pin");
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: path is a valid NUL-terminated path owned throughout the call.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    f.policy("[tools.a]\nexact_from='.pin'");
    f.json(&[], 2);
}

#[test]
fn unreadable_pin_is_an_actionable_configuration_error() {
    let f = Fixture::new();
    let pin = f.root.path().join(".pin");
    fs::write(&pin, "1.2.3\n").unwrap();
    fs::set_permissions(&pin, fs::Permissions::from_mode(0o000)).unwrap();
    // Root can read mode-000 files, so the unreadability case does not exist there.
    if fs::read(&pin).is_ok() {
        return;
    }
    f.policy("[tools.a]\nexact_from='.pin'");
    let json = f.json(&[], 2);
    let error = json["error"].as_str().unwrap();
    assert!(
        error.contains("tools.a") && error.contains(".pin"),
        "{error}"
    );
}

#[test]
fn invalid_utf8_version_output_is_unknown() {
    let f = Fixture::new();
    f.tool(0, "a", "printf '\\377'");
    f.policy("[tools.a]\nminimum='1'");
    let json = f.json(&[], 1);
    assert_eq!(json["tools"][0]["issue"], "unknown_version");
    assert!(json["tools"][0]["actual"].is_null());
}
