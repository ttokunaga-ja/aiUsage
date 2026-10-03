use serde_json::json;
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};
use tempfile::TempDir;

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
