use serde_json::json;
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    sync::Mutex,
};
use tempfile::TempDir;

// A concurrent Linux fork can inherit a copied executable's write descriptor
// before CLOEXEC closes it, causing ETXTBSY. Serialize these copy/spawn tests.
static PROCESS_TESTS: Mutex<()> = Mutex::new(());

fn fixture() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("projects")).unwrap();
    let mut rows = Vec::new();
    for (n, stamp) in [
        "2025-12-31T14:59:59Z",
        "2026-08-31T14:59:59Z",
        "2026-08-31T15:00:00Z",
        "2026-09-30T14:59:59Z",
        "2026-09-30T15:00:00Z",
    ]
    .iter()
    .enumerate()
    {
        rows.push(json!({"type":"assistant", "timestamp":stamp, "requestId":format!("r{n}"), "message":{"id":format!("m{n}"),"model":"claude-opus-5-5", "usage":{"input_tokens":10,"cache_read_input_tokens":20,"cache_creation_input_tokens":30,"cache_creation":{"ephemeral_1h_input_tokens":30},"output_tokens":40}}}).to_string());
    }
    fs::write(dir.path().join("projects/events.jsonl"), rows.join("\n")).unwrap();
    dir
}
fn invoke(config: &TempDir, output: &TempDir, flags: &[&str], reply: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aiUsage"))
        .arg("claude")
        .args(flags)
        .env("CLAUDE_CONFIG_DIR", config.path())
        .current_dir(output.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(reply.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
fn input(path: &std::path::Path) -> u64 {
    let mut reader = csv::Reader::from_path(path.join("usage.csv")).unwrap();
    assert_eq!(reader.headers().unwrap().len(), 6);
    let records: Vec<_> = reader.records().map(Result::unwrap).collect();
    assert_eq!(records.len(), 1);
    records[0][1].parse().unwrap()
}

#[test]
fn dates_always_produce_the_same_six_columns_and_one_row_per_model() {
    let _process_guard = PROCESS_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let config = fixture();
    for (flags, expected) in [
        (vec!["--month", "2026-09"], 20),
        (vec!["--day", "2026-09-30"], 10),
        (vec!["--year", "2026"], 40),
        (vec!["--from", "2026-09-01"], 30),
        (vec!["--to", "2026-09-30"], 40),
        (vec!["--from", "2026-09-01", "--to", "2026-09-30"], 20),
        (vec!["--all"], 50),
        (vec![], 50),
    ] {
        let output = tempfile::tempdir().unwrap();
        let result = invoke(&config, &output, &flags, "");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(input(output.path()), expected);
        assert_eq!(fs::read_dir(output.path()).unwrap().count(), 1);
    }
}

#[test]
fn overwrite_requires_affirmative_input_and_eof_preserves_original() {
    let _process_guard = PROCESS_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let config = fixture();
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("usage.csv");
    fs::write(&path, "original").unwrap();
    for reply in ["", "n\n", "anything\n"] {
        let result = invoke(&config, &output, &["--all"], reply);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("上書き"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    }
    let result = invoke(&config, &output, &["--month", "2026-09"], "y\n");
    assert!(result.status.success());
    assert_eq!(input(output.path()), 20);
}

#[test]
fn invalid_periods_and_removed_options_do_not_create_output() {
    let _process_guard = PROCESS_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let config = fixture();
    for flags in [
        vec!["--month", "2026-09", "--all"],
        vec!["--day", "2026-02-30"],
        vec!["--output", "other.csv"],
        vec!["--group-by", "model"],
        vec!["--from", "2026-09-30", "--to", "2026-09-01"],
    ] {
        let output = tempfile::tempdir().unwrap();
        assert!(!invoke(&config, &output, &flags, "").status.success());
        assert_eq!(fs::read_dir(output.path()).unwrap().count(), 0);
    }
}

#[test]
fn chatgpt_alias_reads_codex_cache_and_anchors_before_period() {
    let _process_guard = PROCESS_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let config = tempfile::tempdir().unwrap();
    fs::create_dir(config.path().join("sessions")).unwrap();
    let u = |input, cached, output| json!({"input_tokens":input,"cached_input_tokens":cached,"output_tokens":output,"reasoning_output_tokens":0,"total_tokens":input+output});
    let first = u(100, 80, 10);
    let rows = [
        json!({"type":"session_meta","payload":{"id":"id","model_provider":"openai"}}),
        json!({"type":"turn_context","payload":{"model":"gpt-6.1-sol"}}),
        json!({"type":"event_msg","timestamp":"2026-08-31T14:59:59Z","payload":{"type":"token_count","info":{"total_token_usage":first,"last_token_usage":first}}}),
        json!({"type":"event_msg","timestamp":"2026-08-31T15:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":u(200,160,20),"last_token_usage":u(100,80,10)}}}),
    ];
    fs::write(
        config
            .path()
            .join("sessions/rollout-11111111-1111-1111-1111-111111111111.jsonl"),
        rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
    )
    .unwrap();
    let output = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_aiUsage"))
        .args(["chatgpt", "--month", "2026-09"])
        .env("CODEX_HOME", config.path())
        .current_dir(output.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut reader = csv::Reader::from_path(output.path().join("usage.csv")).unwrap();
    let row = reader.records().next().unwrap().unwrap();
    assert_eq!(
        row.iter().collect::<Vec<_>>(),
        ["gpt-6.1-sol", "20", "80", "0", "10", "0.000148"]
    );
}

#[test]
fn copied_executable_works_in_a_unicode_directory_without_checkout() {
    let _process_guard = PROCESS_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let config = fixture();
    let output = tempfile::Builder::new()
        .prefix("aiUsage Windows 日本語 ")
        .tempdir()
        .unwrap();
    let copied = output.path().join(if cfg!(windows) {
        "aiUsage.exe"
    } else {
        "aiUsage"
    });
    fs::copy(env!("CARGO_BIN_EXE_aiUsage"), &copied).unwrap();
    for version in ["--version", "-V"] {
        let result = Command::new(&copied)
            .arg(version)
            .current_dir(output.path())
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(
            String::from_utf8(result.stdout).unwrap().trim(),
            concat!("aiUsage ", env!("CARGO_PKG_VERSION"))
        );
    }
    let result = Command::new(&copied)
        .args(["claude", "--all"])
        .env("CLAUDE_CONFIG_DIR", config.path())
        .current_dir(output.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(input(output.path()), 50);
}

#[test]
fn update_rejects_usage_options_without_writing_csv() {
    let _process_guard = PROCESS_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let output = tempfile::tempdir().unwrap();
    for flags in [
        vec!["--all"],
        vec!["--month", "2026-09"],
        vec!["--from", "2026-09-01"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_aiUsage"))
            .arg("update")
            .args(flags)
            .current_dir(output.path())
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("unexpected argument"));
        assert_eq!(fs::read_dir(output.path()).unwrap().count(), 0);
    }
}

fn copied_uninstaller(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.path().join(if cfg!(windows) {
        "日本語 ' $ & uninstall.exe"
    } else {
        "日本語 ' $ & uninstall"
    });
    fs::copy(env!("CARGO_BIN_EXE_aiUsage"), &path).unwrap();
    path
}

fn uninstall(path: &std::path::Path, reply: &str) -> std::process::Output {
    let mut child = Command::new(path)
        .arg("uninstall")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(reply.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn uninstall_cancellation_and_no_bypass_preserve_copied_executable_and_neighbors() {
    let _guard = PROCESS_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let executable = copied_uninstaller(&dir);
    let before = fs::read(&executable).unwrap();
    for reply in ["", "\n", "n\n", "anything\n"] {
        let result = uninstall(&executable, reply);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(stderr.contains(&fs::canonicalize(&executable).unwrap().display().to_string()));
        assert!(stderr.contains("[y/N]"));
        assert_eq!(fs::read(&executable).unwrap(), before);
    }
    for flag in ["--yes", "--force", "-y"] {
        assert!(
            !Command::new(&executable)
                .args(["uninstall", flag])
                .output()
                .unwrap()
                .status
                .success()
        );
        assert_eq!(fs::read(&executable).unwrap(), before);
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn uninstall_affirmative_removes_only_copied_executable() {
    let _guard = PROCESS_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    for reply in ["y\n", "yes\n", "はい\n"] {
        let dir = tempfile::tempdir().unwrap();
        let executable = copied_uninstaller(&dir);
        for name in ["usage.csv", "log.jsonl", "config.json", "other-tool"] {
            fs::write(dir.path().join(name), name).unwrap();
        }
        let result = uninstall(&executable, reply);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        #[cfg(windows)]
        wait_for_uninstall_receipt(&result);
        assert!(!executable.exists());
        for name in ["usage.csv", "log.jsonl", "config.json", "other-tool"] {
            assert_eq!(fs::read_to_string(dir.path().join(name)).unwrap(), name);
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);
    }
}

#[test]
fn uninstall_refuses_a_replacement_made_during_confirmation() {
    use std::io::Read;
    let _guard = PROCESS_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let executable = copied_uninstaller(&dir);
    let mut child = Command::new(&executable)
        .arg("uninstall")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut preview = Vec::new();
    while !preview.ends_with(b"[y/N]: ") {
        let mut byte = [0];
        assert_eq!(stderr.read(&mut byte).unwrap(), 1);
        preview.push(byte[0]);
    }
    let retained = dir.path().join("retained-running-binary");
    fs::rename(&executable, &retained).unwrap();
    fs::write(&executable, b"replacement must survive").unwrap();
    child.stdin.take().unwrap().write_all(b"yes\n").unwrap();
    let mut remainder = String::new();
    stderr.read_to_string(&mut remainder).unwrap();
    assert!(!child.wait().unwrap().success());
    assert!(remainder.contains("変更"));
    assert_eq!(fs::read(&executable).unwrap(), b"replacement must survive");
    assert!(retained.exists());
}

#[cfg(windows)]
fn wait_for_uninstall_receipt(result: &std::process::Output) {
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("削除を予約"));
    let receipt = stderr
        .lines()
        .find_map(|line| {
            line.strip_prefix("結果の記録: ")
                .and_then(|line| line.split_once("（status:").map(|(path, _)| path))
        })
        .unwrap()
        .trim();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    loop {
        let record: serde_json::Value =
            serde_json::from_slice(&fs::read(receipt).unwrap()).unwrap();
        if record["status"] == "deleted" {
            break;
        }
        assert_ne!(record["status"], "failed", "{record}");
        assert!(std::time::Instant::now() < deadline, "{record}");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    fs::remove_dir_all(std::path::Path::new(receipt).parent().unwrap()).unwrap();
}

#[cfg(windows)]
#[test]
fn uninstall_ignores_inherited_incompatible_powershell_modules() {
    let _guard = PROCESS_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let executable = copied_uninstaller(&dir);
    let modules = dir.path().join("shadow-modules");
    let utility = modules.join("Microsoft.PowerShell.Utility");
    fs::create_dir_all(&utility).unwrap();
    fs::write(utility.join("Microsoft.PowerShell.Utility.psd1"),
        "@{ RootModule='Microsoft.PowerShell.Utility.psm1'; ModuleVersion='7.0.0'; GUID='159b5e0e-66b7-4e52-ab34-c7c9e27c91bd'; FunctionsToExport=@('Get-FileHash','ConvertTo-Json'); PowerShellVersion='5.1' }").unwrap();
    fs::write(utility.join("Microsoft.PowerShell.Utility.psm1"),
        "function Get-FileHash { throw 'Incompatible inherited module selected' }; function ConvertTo-Json { throw 'Incompatible inherited module selected' }; Export-ModuleMember -Function Get-FileHash,ConvertTo-Json").unwrap();
    let mut inherited = modules.into_os_string();
    inherited.push(";");
    inherited.push(
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/Modules"),
    );
    let mut child = Command::new(&executable)
        .arg("uninstall")
        .env("PSModulePath", inherited)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"yes\n").unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    wait_for_uninstall_receipt(&result);
    assert!(!executable.exists());
    assert!(utility.join("Microsoft.PowerShell.Utility.psd1").exists());
}
