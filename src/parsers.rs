//! Read-only, streaming extraction. Only usage metadata is retained in memory.
use crate::model::{Event, Provider, Scan, Tokens};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MARKERS: [&[u8]; 4] = [
    b"\"session_meta\"",
    b"\"turn_context\"",
    b"\"token_count\"",
    b"\"model/rerouted\"",
];

#[derive(Debug)]
struct Snapshot {
    execution: String,
    metadata_origin: Option<String>,
    timestamp: DateTime<Utc>,
    model: String,
    provider: String,
    total: [u64; 6],
    last: Option<[u64; 6]>,
    order: usize,
}

pub fn scan(provider: Provider, roots: &[PathBuf]) -> Result<Scan> {
    let mut paths = BTreeSet::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry.with_context(|| format!("traverse {}", root.display()))?;
            if entry.file_type().is_file() && entry.path().extension().is_some_and(|x| x == "jsonl")
            {
                paths.insert(fs::canonicalize(entry.path())?);
            }
        }
    }
    let mut result = Scan::default();
    let mut claude_calls = BTreeMap::<String, Event>::new();
    let mut snapshots = Vec::new();
    for path in paths {
        let file = File::open(&path).with_context(|| format!("read {}", path.display()))?;
        let before = file.metadata()?;
        // A scan-start byte boundary allows active append-only logs to remain
        // readable without including requests written during this file's scan.
        let mut reader = BufReader::new(file.take(before.len()));
        let mut raw = Vec::new();
        let mut line = 0;
        let mut model = "unknown".to_string();
        let mut model_provider = "unknown".to_string();
        let mut execution = execution_id(&path);
        let mut metadata_origin = None;
        let mut first_meta = true;
        loop {
            raw.clear();
            if reader
                .read_until(b'\n', &mut raw)
                .with_context(|| format!("read {} line {}", path.display(), line + 1))?
                == 0
            {
                break;
            }
            line += 1;
            if provider == Provider::Chatgpt
                && !MARKERS
                    .iter()
                    .any(|m| memchr::memmem::find(&raw, m).is_some())
            {
                continue;
            }
            let row: Value = match serde_json::from_slice(&raw) {
                Ok(row) => row,
                Err(_) if !raw.ends_with(b"\n") => {
                    result.excluded += 1;
                    continue;
                }
                Err(_) => {
                    result.malformed += 1;
                    continue;
                }
            };
            match provider {
                Provider::Claude => read_claude(&row, &path, line, &mut claude_calls, &mut result),
                Provider::Chatgpt => {
                    let payload = &row["payload"];
                    match row["type"].as_str() {
                        Some("session_meta") if first_meta => {
                            metadata_origin = payload["id"]
                                .as_str()
                                .filter(|id| !id.is_empty())
                                .map(str::to_owned);
                            if execution.is_empty() {
                                execution = string(&payload["id"], &path.to_string_lossy());
                            }
                            model_provider = string(&payload["model_provider"], "unknown");
                            if let Some(m) = payload["model"].as_str() {
                                model = m.to_string();
                            }
                            first_meta = false;
                        }
                        Some("turn_context") => {
                            if let Some(m) = payload["model"].as_str() {
                                model = m.to_string();
                            }
                            if let Some(p) = payload["model_provider"].as_str() {
                                model_provider = p.to_string();
                            }
                        }
                        Some("event_msg") if payload["type"] == "model/rerouted" => {
                            if let Some(m) = payload["to_model"]
                                .as_str()
                                .or_else(|| payload["to"].as_str())
                            {
                                model = m.to_string();
                            }
                            if let Some(p) = payload["to_provider"]
                                .as_str()
                                .or_else(|| payload["model_provider"].as_str())
                            {
                                model_provider = p.to_string();
                            }
                        }
                        Some("event_msg") if payload["type"] == "token_count" => {
                            let info = &payload["info"];
                            let Some(timestamp) = timestamp(&row["timestamp"]) else {
                                result.excluded += 1;
                                continue;
                            };
                            let Some(total) = counters(&info["total_token_usage"]) else {
                                result.excluded += 1;
                                continue;
                            };
                            snapshots.push(Snapshot {
                                execution: if execution.is_empty() {
                                    path.to_string_lossy().into_owned()
                                } else {
                                    execution.clone()
                                },
                                metadata_origin: metadata_origin.clone(),
                                timestamp,
                                model: model.clone(),
                                provider: model_provider.clone(),
                                total,
                                last: counters(&info["last_token_usage"]),
                                order: snapshots.len(),
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
        let after = fs::metadata(&path)?;
        if !source_stable(&before, &after)? {
            result.unstable += 1;
        }
        result.files += 1;
    }
    match provider {
        Provider::Claude => {
            for mut event in claude_calls.into_values() {
                let t = &event.tokens;
                let context = t
                    .input
                    .checked_add(t.read)
                    .and_then(|n| n.checked_add(t.write));
                let Some(context) = context else {
                    result.excluded += 1;
                    continue;
                };
                if t.write_5m
                    .checked_add(t.write_1h)
                    .is_none_or(|n| n > t.write)
                {
                    result.excluded += 1;
                    continue;
                }
                event.context_input = context;
                result.events.push(event);
            }
        }
        Provider::Chatgpt => classify_snapshots(snapshots, &mut result),
    }
    result.events.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then(a.provider.cmp(&b.provider))
            .then(a.model.cmp(&b.model))
    });
    Ok(result)
}

fn source_stable(before: &fs::Metadata, after: &fs::Metadata) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Ok(false);
        }
    }
    Ok(after.len() >= before.len()
        && (after.len() > before.len() || before.modified()? == after.modified()?))
}

fn string(v: &Value, fallback: &str) -> String {
    v.as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback)
        .to_string()
}
fn timestamp(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}
fn optional_count(v: &Value) -> Option<u64> {
    if v.is_null() { Some(0) } else { v.as_u64() }
}

fn read_claude(
    row: &Value,
    path: &Path,
    line: usize,
    calls: &mut BTreeMap<String, Event>,
    scan: &mut Scan,
) {
    if row["type"] != "assistant" {
        return;
    }
    let message = &row["message"];
    let usage = &message["usage"];
    if !usage.is_object() || message["model"] == "<synthetic>" {
        return;
    }
    let Some(stamp) = timestamp(&row["timestamp"]) else {
        scan.excluded += 1;
        return;
    };
    let model = string(&message["model"], "unknown");
    let values = [
        "input_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
        "output_tokens",
    ]
    .map(|k| optional_count(&usage[k]));
    let [Some(input), Some(read), Some(write), Some(output)] = values else {
        scan.excluded += 1;
        return;
    };
    let (Some(write_5m), Some(write_1h)) = (
        optional_count(&usage["cache_creation"]["ephemeral_5m_input_tokens"]),
        optional_count(&usage["cache_creation"]["ephemeral_1h_input_tokens"]),
    ) else {
        scan.excluded += 1;
        return;
    };
    if write_5m.checked_add(write_1h).is_none_or(|n| n > write) {
        scan.excluded += 1;
        return;
    }
    let Some(context_input) = input.checked_add(read).and_then(|n| n.checked_add(write)) else {
        scan.excluded += 1;
        return;
    };
    let request = row["requestId"].as_str().filter(|s| !s.is_empty());
    let id = message["id"].as_str().filter(|s| !s.is_empty());
    let key = if request.is_some() || id.is_some() {
        serde_json::to_string(&(request, id, &model)).unwrap()
    } else {
        serde_json::to_string(&(
            string(&row["uuid"], &format!("{}:{line}", path.display())),
            &model,
        ))
        .unwrap()
    };
    let event = Event {
        timestamp: stamp,
        model,
        provider: "anthropic".into(),
        tokens: Tokens {
            input,
            read,
            write,
            output,
            write_5m,
            write_1h,
        },
        fast: usage["speed"] == "fast",
        us_geo: usage["inference_geo"] == "us",
        context_input,
    };
    if let Some(old) = calls.get_mut(&key) {
        scan.duplicates += 1;
        old.tokens.input = old.tokens.input.max(input);
        old.tokens.read = old.tokens.read.max(read);
        old.tokens.write = old.tokens.write.max(write);
        old.tokens.output = old.tokens.output.max(output);
        old.tokens.write_5m = old.tokens.write_5m.max(write_5m);
        old.tokens.write_1h = old.tokens.write_1h.max(write_1h);
        old.timestamp = old.timestamp.max(stamp);
        old.fast |= event.fast;
        old.us_geo |= event.us_geo;
        old.context_input = old
            .tokens
            .input
            .saturating_add(old.tokens.read)
            .saturating_add(old.tokens.write);
    } else {
        calls.insert(key, event);
    }
}

/// Filenames, rather than copied session_meta IDs, identify fork executions.
fn execution_id(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let Some(start) = stem.len().checked_sub(36) else {
        return String::new();
    };
    let Some(suffix) = stem.get(start..) else {
        return String::new();
    };
    if suffix.bytes().enumerate().all(|(i, b)| {
        if [8, 13, 18, 23].contains(&i) {
            b == b'-'
        } else {
            b.is_ascii_hexdigit()
        }
    }) {
        suffix.to_ascii_lowercase()
    } else {
        String::new()
    }
}

fn counters(v: &Value) -> Option<[u64; 6]> {
    if !v.is_object() {
        return None;
    }
    Some([
        v["input_tokens"].as_u64()?,
        v["cached_input_tokens"].as_u64()?,
        match v.get("cache_write_input_tokens") {
            None => 0,
            Some(write) => write.as_u64()?,
        },
        v["output_tokens"].as_u64()?,
        v["reasoning_output_tokens"].as_u64()?,
        v["total_tokens"].as_u64()?,
    ])
}
fn valid(v: &[u64; 6]) -> bool {
    v[1].checked_add(v[2]).is_some_and(|cache| cache <= v[0])
        && v[4] <= v[3]
        && v[0].checked_add(v[3]) == Some(v[5])
}
fn classify_snapshots(mut snapshots: Vec<Snapshot>, scan: &mut Scan) {
    snapshots.sort_by(|a, b| {
        a.execution
            .cmp(&b.execution)
            .then(a.timestamp.cmp(&b.timestamp))
            .then(a.order.cmp(&b.order))
    });
    // Compare only against the explicitly named origin execution. Equal counters
    // in unrelated executions are independent usage, even at the same timestamp.
    let origin_index: HashSet<_> = snapshots
        .iter()
        .map(|s| snapshot_key(s, &s.execution))
        .collect();
    let mut seen = HashSet::new();
    let mut previous: Option<[u64; 6]> = None;
    let mut execution = String::new();
    for s in &snapshots {
        if s.execution != execution {
            execution.clone_from(&s.execution);
            previous = None;
        }
        let key = snapshot_key(s, &s.execution);
        if !seen.insert(key) {
            scan.duplicates += 1;
            continue;
        }
        if !valid(&s.total) {
            scan.excluded += 1;
            continue;
        }
        if s.metadata_origin.as_deref().is_some_and(|origin| {
            origin != s.execution && origin_index.contains(&snapshot_key(s, origin))
        }) {
            // The copied prefix is not a new request, but remains the anchor
            // for this child's own subsequent cumulative counter deltas.
            previous = Some(s.total);
            scan.duplicates += 1;
            continue;
        }
        let values = match previous {
            None => {
                if s.last == Some(s.total) {
                    Some(s.total)
                } else {
                    None
                }
            }
            Some(p) if p == s.total => {
                scan.duplicates += 1;
                continue;
            }
            Some(p) if s.total.iter().zip(p).any(|(a, b)| *a < b) => {
                if s.last == Some(s.total) {
                    Some(s.total)
                } else {
                    None
                }
            }
            Some(p) => {
                let delta = std::array::from_fn(|i| s.total[i] - p[i]);
                if s.last == Some(delta) {
                    Some(delta)
                } else {
                    None
                }
            }
        };
        // Ambiguous initial/reset snapshots still anchor the next valid delta.
        previous = Some(s.total);
        let Some(v) = values.filter(valid) else {
            scan.excluded += 1;
            continue;
        };
        scan.events.push(Event {
            timestamp: s.timestamp,
            model: s.model.clone(),
            provider: s.provider.clone(),
            tokens: Tokens {
                input: v[0] - v[1] - v[2],
                read: v[1],
                write: v[2],
                output: v[3],
                ..Tokens::default()
            },
            fast: false,
            us_geo: false,
            context_input: v[0],
        });
    }
}

type SnapshotKey<'a> = (
    &'a str,
    DateTime<Utc>,
    [u64; 6],
    Option<[u64; 6]>,
    &'a str,
    &'a str,
);

fn snapshot_key<'a>(snapshot: &'a Snapshot, execution: &'a str) -> SnapshotKey<'a> {
    (
        execution,
        snapshot.timestamp,
        snapshot.total,
        snapshot.last,
        &snapshot.provider,
        &snapshot.model,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, rows: &[Value]) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(
            &path,
            rows.iter().map(|v| format!("{v}\n")).collect::<String>(),
        )
        .unwrap();
        path
    }
    fn usage(input: u64, read: u64, output: u64) -> Value {
        json!({"input_tokens":input,"cached_input_tokens":read,"output_tokens":output,"reasoning_output_tokens":output / 2,"total_tokens":input+output})
    }
    fn count(stamp: &str, total: Value, last: Value) -> Value {
        json!({"timestamp":stamp,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":total,"last_token_usage":last}}})
    }
    fn meta() -> Value {
        json!({"type":"session_meta","payload":{"id":"copied-parent","model_provider":"openai"}})
    }
    fn context() -> Value {
        json!({"type":"turn_context","payload":{"model":"future-unknown-model"}})
    }

    #[test]
    fn claude_streaming_snapshots_merge_cache_and_latest_timestamp() {
        let dir = TempDir::new().unwrap();
        let assistant = |stamp: &str, output: u64| json!({"type":"assistant","requestId":"request","timestamp":stamp,"message":{"id":"message","model":"unknown-claude","usage":{"input_tokens":10,"cache_read_input_tokens":20,"cache_creation_input_tokens":30,"cache_creation":{"ephemeral_5m_input_tokens":10,"ephemeral_1h_input_tokens":20},"output_tokens":output,"speed":"fast","inference_geo":"us"}}});
        write(
            &dir,
            "a.jsonl",
            &[
                assistant("2026-08-31T23:59:59Z", 2),
                assistant("2026-09-01T00:00:01Z", 5),
                json!({"type":"assistant","message":{"model":"<synthetic>","usage":{}}}),
            ],
        );
        let s = scan(Provider::Claude, &[dir.path().into()]).unwrap();
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.duplicates, 1);
        let e = &s.events[0];
        assert_eq!(
            e.tokens,
            Tokens {
                input: 10,
                read: 20,
                write: 30,
                output: 5,
                write_5m: 10,
                write_1h: 20
            }
        );
        assert_eq!(e.context_input, 60);
        assert!(e.fast && e.us_geo);
        assert_eq!(e.timestamp.to_rfc3339(), "2026-09-01T00:00:01+00:00");
    }

    #[test]
    fn codex_repeat_delta_and_period_boundary_anchor() {
        let dir = TempDir::new().unwrap();
        let first = usage(100, 20, 10);
        let next = usage(150, 30, 20);
        write(
            &dir,
            "rollout-11111111-1111-1111-1111-111111111111.jsonl",
            &[
                meta(),
                context(),
                count("2026-08-31T14:59:00Z", first.clone(), first.clone()),
                count("2026-08-31T14:59:30Z", first.clone(), first),
                count("2026-08-31T15:01:00Z", next, usage(50, 10, 10)),
            ],
        );
        let s = scan(Provider::Chatgpt, &[dir.path().into()]).unwrap();
        assert_eq!(s.events.len(), 2);
        assert_eq!(s.duplicates, 1);
        assert_eq!(s.excluded, 0);
        assert_eq!(s.events[1].tokens.input, 40);
        assert_eq!(s.events[1].tokens.output, 10); // Reasoning is already part of output.
        assert_eq!(s.events[1].model, "future-unknown-model");
        assert_eq!(s.events[1].context_input, 50);
    }

    #[test]
    fn codex_ambiguous_snapshots_anchor_and_resets_require_last_match() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "rollout-11111111-1111-1111-1111-111111111111.jsonl",
            &[
                meta(),
                context(),
                count(
                    "2026-09-01T00:00:00Z",
                    usage(100, 20, 10),
                    usage(50, 10, 10),
                ),
                count(
                    "2026-09-01T00:00:01Z",
                    usage(150, 30, 20),
                    usage(50, 10, 10),
                ),
                count("2026-09-01T00:00:02Z", usage(20, 0, 10), usage(20, 0, 10)),
                count("2026-09-01T00:00:03Z", usage(10, 0, 2), usage(5, 0, 2)),
                count("2026-09-01T00:00:04Z", usage(20, 0, 4), usage(10, 0, 2)),
            ],
        );
        let s = scan(Provider::Chatgpt, &[dir.path().into()]).unwrap();
        assert_eq!(s.events.len(), 3);
        assert_eq!(s.excluded, 2);
    }

    #[test]
    fn execution_identity_preserves_children_and_deduplicates_rollout_copies() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("archive")).unwrap();
        let rows = [
            meta(),
            context(),
            count(
                "2026-09-01T00:00:00Z",
                usage(100, 20, 10),
                usage(100, 20, 10),
            ),
            count(
                "2026-09-01T00:00:01Z",
                usage(150, 30, 20),
                usage(50, 10, 10),
            ),
        ];
        let path = write(
            &dir,
            "rollout-11111111-1111-1111-1111-111111111111.jsonl",
            &rows,
        );
        fs::copy(
            &path,
            dir.path().join("archive").join(path.file_name().unwrap()),
        )
        .unwrap();
        write(
            &dir,
            "rollout-22222222-2222-2222-2222-222222222222.jsonl",
            &rows,
        );
        let s = scan(Provider::Chatgpt, &[dir.path().into()]).unwrap();
        assert_eq!(s.events.len(), 4);
        assert_eq!(s.duplicates, 2);
        assert_eq!(s.events.iter().map(|e| e.tokens.input).sum::<u64>(), 240);
    }

    #[test]
    fn malformed_body_only_lines_are_not_parsed_and_bad_usage_is_excluded() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "rollout-11111111-1111-1111-1111-111111111111.jsonl",
            &[
                meta(),
                context(),
                count("2026-09-01T00:00:00Z", usage(10, 20, 2), usage(10, 20, 2)),
            ],
        );
        use std::io::Write;
        let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
        file.write_all(b"not json body only\n{broken \"token_count\"\n")
            .unwrap();
        let s = scan(Provider::Chatgpt, &[dir.path().into()]).unwrap();
        assert_eq!(s.malformed, 1);
        assert_eq!(s.excluded, 1);
        assert!(s.events.is_empty());
    }

    #[test]
    fn codex_explicit_cache_write_and_missing_last_are_conservative() {
        let dir = TempDir::new().unwrap();
        let mut first = usage(100, 20, 10);
        first["cache_write_input_tokens"] = json!(10);
        let mut next = usage(150, 30, 20);
        next["cache_write_input_tokens"] = json!(15);
        write(
            &dir,
            "rollout-11111111-1111-1111-1111-111111111111.jsonl",
            &[
                meta(),
                context(),
                count("2026-09-01T00:00:00Z", first.clone(), first),
                count("2026-09-01T00:00:01Z", next, Value::Null),
            ],
        );
        let s = scan(Provider::Chatgpt, &[dir.path().into()]).unwrap();
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.excluded, 1);
        assert_eq!(s.events[0].tokens.input, 70);
        assert_eq!(s.events[0].tokens.write, 10);
        let mut negative = usage(10, 0, 2);
        negative["input_tokens"] = json!(-1);
        assert!(counters(&negative).is_none());
        let mut null_write = usage(10, 0, 2);
        null_write["cache_write_input_tokens"] = Value::Null;
        assert!(counters(&null_write).is_none());
    }

    #[test]
    fn copied_fork_prefix_requires_exact_explicit_origin() {
        let parent = "11111111-1111-1111-1111-111111111111";
        let child = "22222222-2222-2222-2222-222222222222";
        for (origin, inherited) in [
            (Some(parent), true),
            (Some(child), false),
            (None, false),
            (Some("absent-parent"), false),
        ] {
            let dir = TempDir::new().unwrap();
            let prefix = count(
                "2026-09-01T00:00:00Z",
                usage(100, 20, 10),
                usage(100, 20, 10),
            );
            write(
                &dir,
                &format!("rollout-{parent}.jsonl"),
                &[
                    json!({"type":"session_meta","payload":{"id":parent,"model_provider":"openai"}}),
                    context(),
                    prefix.clone(),
                ],
            );
            write(
                &dir,
                &format!("rollout-{child}.jsonl"),
                &[
                    json!({"type":"session_meta","payload":{"id":origin,"model_provider":"openai"}}),
                    context(),
                    prefix,
                    count(
                        "2026-09-01T00:00:01Z",
                        usage(150, 30, 20),
                        usage(50, 10, 10),
                    ),
                ],
            );
            let s = scan(Provider::Chatgpt, &[dir.path().into()]).unwrap();
            assert_eq!(s.events.len(), if inherited { 2 } else { 3 });
            assert_eq!(
                s.events.iter().map(|e| e.tokens.input).sum::<u64>(),
                if inherited { 120 } else { 200 }
            );
            assert_eq!(s.duplicates, usize::from(inherited));
        }
    }

    #[test]
    fn source_growth_is_allowed_but_truncation_and_rotation_are_unstable() {
        use std::io::Write;
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "source.jsonl", &[meta()]);
        let before = fs::metadata(&path).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"more\n")
            .unwrap();
        assert!(source_stable(&before, &fs::metadata(&path).unwrap()).unwrap());
        fs::write(&path, b"short").unwrap();
        assert!(!source_stable(&before, &fs::metadata(&path).unwrap()).unwrap());
        #[cfg(unix)]
        {
            let replacement = dir.path().join("replacement");
            fs::write(&replacement, vec![b'x'; before.len() as usize]).unwrap();
            fs::rename(replacement, &path).unwrap();
            assert!(!source_stable(&before, &fs::metadata(&path).unwrap()).unwrap());
        }
    }

    #[test]
    fn incomplete_scan_boundary_is_excluded_instead_of_malformed() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("source.jsonl"),
            b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"",
        )
        .unwrap();
        let s = scan(Provider::Chatgpt, &[dir.path().into()]).unwrap();
        assert_eq!(s.excluded, 1);
        assert_eq!(s.malformed, 0);
        assert_eq!(s.unstable, 0);
    }
}
