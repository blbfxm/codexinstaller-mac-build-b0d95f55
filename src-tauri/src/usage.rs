//! Read-only, on-demand accounting of the current Codex home's JSONL sessions.
use chrono::{DateTime, Duration, Local};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Instant, SystemTime},
};
use tauri::State;

const MAX_LINE: usize = 2 * 1024 * 1024;
const MAX_FILES: usize = 20_000;
const MAX_RECORDS: usize = 400_000;

#[derive(Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    input: u64,
    cached: u64,
    output: u64,
    reasoning: u64,
    total: u64,
}
impl Tokens {
    fn parse(value: &Value) -> Option<Self> {
        let input = value["input_tokens"].as_u64()?;
        let output = value["output_tokens"].as_u64()?;
        Some(Self {
            input,
            output,
            cached: value["cached_input_tokens"]
                .as_u64()
                .unwrap_or(0)
                .min(input),
            reasoning: value["reasoning_output_tokens"]
                .as_u64()
                .unwrap_or(0)
                .min(output),
            total: input.saturating_add(output),
        })
    }
    fn add(&mut self, other: Self) {
        self.input = self.input.saturating_add(other.input);
        self.cached = self.cached.saturating_add(other.cached);
        self.output = self.output.saturating_add(other.output);
        self.reasoning = self.reasoning.saturating_add(other.reasoning);
        self.total = self.input.saturating_add(self.output);
    }
    fn delta(self, before: Self) -> Option<Self> {
        if self.input < before.input || self.output < before.output {
            return None;
        }
        let input = self.input - before.input;
        let output = self.output - before.output;
        Some(Self {
            input,
            output,
            cached: self.cached.saturating_sub(before.cached).min(input),
            reasoning: self.reasoning.saturating_sub(before.reasoning).min(output),
            total: input.saturating_add(output),
        })
    }
}

#[derive(Clone)]
struct Record {
    session: String,
    time: i64,
    model: String,
    total: Option<Tokens>,
    last: Option<Tokens>,
    request_id: Option<String>,
}
struct FileData {
    length: u64,
    modified: Option<SystemTime>,
    records: Vec<Record>,
    partial: bool,
}
#[derive(Default)]
struct Index {
    home: PathBuf,
    files: HashMap<PathBuf, FileData>,
    refreshed: Option<Instant>,
    partial: bool,
}
#[derive(Default)]
pub struct UsageState(std::sync::Arc<Mutex<Index>>);

fn collect(directory: &Path, paths: &mut Vec<PathBuf>, partial: &mut bool, depth: usize) {
    if depth > 8 || paths.len() >= MAX_FILES {
        *partial = true;
        return;
    }
    let entries = match fs::read_dir(directory) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            *partial = true;
            return;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            *partial = true;
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            *partial = true;
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            collect(&path, paths, partial, depth + 1);
        } else if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            if paths.len() >= MAX_FILES {
                *partial = true;
                return;
            }
            paths.push(path);
        } else if path.extension().and_then(|s| s.to_str()) == Some("zst") {
            *partial = true;
        }
    }
}

fn read_file(path: &Path) -> std::io::Result<FileData> {
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    let mut session = path.to_string_lossy().into_owned();
    let mut model = "未记录模型".to_string();
    let mut records = Vec::new();
    let mut partial = false;
    loop {
        buffer.clear();
        let count = reader
            .by_ref()
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut buffer)?;
        if count == 0 {
            break;
        }
        if count > MAX_LINE {
            partial = true;
            while !buffer.ends_with(b"\n") {
                buffer.clear();
                if reader
                    .by_ref()
                    .take(MAX_LINE as u64)
                    .read_until(b'\n', &mut buffer)?
                    == 0
                {
                    break;
                }
            }
            continue;
        }
        // Ignore prompts and tool results without deserializing their content.
        if !buffer.windows(11).any(|w| w == b"token_count")
            && !buffer
                .windows(12)
                .any(|w| w == b"session_meta" || w == b"turn_context")
        {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&buffer) else {
            partial = true;
            continue;
        };
        let payload = &value["payload"];
        match value["type"].as_str().unwrap_or_default() {
            "session_meta" => {
                if let Some(id) = payload["id"].as_str() {
                    session = id.into();
                }
            }
            "turn_context" => {
                if let Some(name) = payload["model"].as_str() {
                    model = name.into();
                }
            }
            "event_msg" if payload["type"] == "token_count" && payload["info"].is_object() => {
                let Some(time) = value["timestamp"]
                    .as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.timestamp_millis())
                else {
                    partial = true;
                    continue;
                };
                records.push(Record {
                    session: session.clone(),
                    time,
                    model: model.clone(),
                    total: Tokens::parse(&payload["info"]["total_token_usage"]),
                    last: Tokens::parse(&payload["info"]["last_token_usage"]),
                    request_id: payload["request_id"]
                        .as_str()
                        .or_else(|| payload["info"]["request_id"].as_str())
                        .or_else(|| value["request_id"].as_str())
                        .filter(|id| id.len() <= 256)
                        .map(str::to_owned),
                });
                if records.len() >= MAX_RECORDS {
                    partial = true;
                    break;
                }
            }
            _ => {}
        }
    }
    Ok(FileData {
        length: metadata.len(),
        modified: metadata.modified().ok(),
        records,
        partial,
    })
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    period: String,
    tokens: Tokens,
    sessions: usize,
    requests: usize,
    days: Vec<Group>,
    models: Vec<Group>,
    complete: bool,
    files: usize,
    updated_at: String,
    notice: String,
    #[serde(skip)]
    logs: Option<UsageLogPage>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEntry {
    time: String,
    session_id: String,
    model: String,
    request_id: Option<String>,
    tokens: Tokens,
    note: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLogPage {
    page: usize,
    page_size: usize,
    total: usize,
    total_pages: usize,
    entries: Vec<UsageEntry>,
    complete: bool,
    notice: String,
    updated_at: String,
}
#[derive(Serialize, Default)]
struct Group {
    name: String,
    tokens: Tokens,
    requests: usize,
}

fn summarize(records: Vec<Record>, period: &str, partial: bool, files: usize) -> Summary {
    summarize_with_logs(records, period, partial, files, None)
}
fn summarize_with_logs(
    records: Vec<Record>,
    period: &str,
    mut partial: bool,
    files: usize,
    log_page: Option<usize>,
) -> Summary {
    let today = Local::now().date_naive();
    let start = match period {
        "today" => Some(today),
        "7d" => Some(today - Duration::days(6)),
        "30d" => Some(today - Duration::days(29)),
        _ => None,
    };
    let mut sorted = records;
    sorted.sort_by(|a, b| {
        (&a.session, a.time, a.total.map(|t| (t.input, t.output))).cmp(&(
            &b.session,
            b.time,
            b.total.map(|t| (t.input, t.output)),
        ))
    });
    let mut previous: HashMap<String, Tokens> = HashMap::new();
    let mut seen = HashSet::new();
    let mut sessions = HashSet::new();
    let mut days: BTreeMap<String, Group> = BTreeMap::new();
    let mut models: BTreeMap<String, Group> = BTreeMap::new();
    let mut logs = Vec::new();
    let mut summary = Summary {
        period: period.into(),
        files,
        updated_at: Local::now().to_rfc3339(),
        ..Summary::default()
    };
    for record in sorted {
        let signature = (
            record.session.clone(),
            record.time,
            record.total.map(|t| (t.input, t.output)),
            record.last.map(|t| (t.input, t.output)),
        );
        if !seen.insert(signature) {
            continue;
        }
        let mut note = String::new();
        let tokens = if let Some(total) = record.total {
            let before = previous.insert(record.session.clone(), total);
            match before {
                Some(before) => match total.delta(before) {
                    Some(delta) => delta,
                    None => {
                        partial = true;
                        note = "会话计数重置，显示本次记录用量".into();
                        record.last.unwrap_or_default()
                    }
                },
                None => match record.last {
                    Some(last) => {
                        if last != total {
                            partial = true;
                            note = "历史累计基线缺失，显示本次记录用量".into();
                        }
                        last
                    }
                    None => {
                        partial = true;
                        Tokens::default()
                    }
                },
            }
        } else {
            partial = true;
            note = "无累计计数，显示本次记录用量".into();
            record.last.unwrap_or_default()
        };
        if tokens.total == 0 {
            continue;
        }
        let Some(date) = DateTime::from_timestamp_millis(record.time)
            .map(|t| t.with_timezone(&Local).date_naive())
        else {
            partial = true;
            continue;
        };
        if date > today || start.is_some_and(|start| date < start) {
            continue;
        }
        if log_page.is_some() {
            logs.push((
                record.time,
                UsageEntry {
                    time: DateTime::from_timestamp_millis(record.time)
                        .unwrap()
                        .to_rfc3339(),
                    session_id: record.session.clone(),
                    model: record.model.clone(),
                    request_id: record.request_id,
                    tokens,
                    note,
                },
            ));
        }
        sessions.insert(record.session);
        summary.tokens.add(tokens);
        summary.requests += 1;
        let day_name = date.to_string();
        for (map, name) in [(&mut days, day_name), (&mut models, record.model)] {
            let group = map.entry(name.clone()).or_insert_with(|| Group {
                name,
                ..Group::default()
            });
            group.tokens.add(tokens);
            group.requests += 1;
        }
    }
    summary.sessions = sessions.len();
    summary.days = days.into_values().collect();
    summary.models = models.into_values().collect();
    summary
        .models
        .sort_by_key(|g| std::cmp::Reverse(g.tokens.total));
    summary.complete = !partial;
    summary.notice = if partial { "部分会话计数缺失、重置或文件不可读取；当前显示可确认的用量，不等同于平台扣费账单。" } else { "根据本机 Codex 会话统计；缓存输入包含在输入 Token 中，推理包含在输出 Token 中。平台扣费以实际账单为准。" }.into();
    if let Some(page) = log_page {
        logs.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.session_id.cmp(&b.1.session_id))
        });
        let total = logs.len();
        let total_pages = total.div_ceil(20).max(1);
        let page = page.max(1).min(total_pages);
        let entries = logs
            .into_iter()
            .skip((page - 1) * 20)
            .take(20)
            .map(|(_, entry)| entry)
            .collect();
        summary.logs = Some(UsageLogPage {
            page,
            page_size: 20,
            total,
            total_pages,
            entries,
            complete: summary.complete,
            notice: summary.notice.clone(),
            updated_at: summary.updated_at.clone(),
        });
    }
    summary
}

#[tauri::command]
pub async fn codex_usage(state: State<'_, UsageState>, period: String) -> Result<Summary, String> {
    load_usage(&state, period, None).await
}
#[tauri::command]
pub async fn codex_usage_logs(
    state: State<'_, UsageState>,
    period: String,
    page: usize,
) -> Result<UsageLogPage, String> {
    load_usage(&state, period, Some(page))
        .await?
        .logs
        .ok_or_else(|| "使用日志读取失败".into())
}
async fn load_usage(
    state: &UsageState,
    period: String,
    log_page: Option<usize>,
) -> Result<Summary, String> {
    if !matches!(period.as_str(), "today" | "7d" | "30d" | "all") {
        return Err("用量统计时段无效".into());
    }
    let home = super::codex_home_dir()?;
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<Summary, String> {
        let mut index = shared.lock().map_err(|_| "用量索引繁忙")?;
        if index.home != home {
            *index = Index {
                home: home.clone(),
                ..Index::default()
            };
        }
        let mut partial = index.partial;
        if index
            .refreshed
            .is_none_or(|time| time.elapsed().as_secs() >= 10)
        {
            partial = false;
            let mut paths = Vec::new();
            collect(&home.join("sessions"), &mut paths, &mut partial, 0);
            collect(&home.join("archived_sessions"), &mut paths, &mut partial, 0);
            let present: HashSet<_> = paths.iter().cloned().collect();
            index.files.retain(|p, _| present.contains(p));
            for path in paths {
                let Ok(meta) = fs::metadata(&path) else {
                    partial = true;
                    continue;
                };
                if index
                    .files
                    .get(&path)
                    .is_some_and(|f| f.length == meta.len() && f.modified == meta.modified().ok())
                {
                    continue;
                }
                match read_file(&path) {
                    Ok(file) => {
                        index.files.insert(path, file);
                    }
                    Err(_) => partial = true,
                }
            }
            index.refreshed = Some(Instant::now());
            index.partial = partial;
        }
        let mut records = Vec::new();
        for file in index.files.values() {
            partial |= file.partial;
            if records.len() + file.records.len() > MAX_RECORDS {
                partial = true;
                continue;
            }
            records.extend(file.records.clone());
        }
        let summary = if log_page.is_some() {
            summarize_with_logs(records, &period, partial, index.files.len(), log_page)
        } else {
            summarize(records, &period, partial, index.files.len())
        };
        Ok(summary)
    })
    .await
    .map_err(|e| format!("用量统计失败：{e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(time: i64, input: u64, output: u64, last_input: u64, last_output: u64) -> Record {
        let total = Tokens {
            input,
            output,
            cached: input / 2,
            total: input + output,
            ..Tokens::default()
        };
        Record {
            session: "test".into(),
            time,
            model: "test-model".into(),
            total: Some(total),
            last: Some(Tokens {
                input: last_input,
                output: last_output,
                total: last_input + last_output,
                ..Tokens::default()
            }),
            request_id: None,
        }
    }
    #[test]
    fn logs_are_reverse_chronological_and_exactly_twenty_per_page() {
        let now = Local::now().timestamp_millis();
        let mut records = Vec::new();
        for i in 1..=45 {
            records.push(record(
                now - 5000 + i * 10,
                (i * 10) as u64,
                i as u64,
                10,
                1,
            ));
        }
        records.push(records[0].clone());
        let report = summarize_with_logs(records.clone(), "all", false, 1, Some(1));
        let page = report.logs.unwrap();
        assert_eq!(page.total, 45);
        assert_eq!(page.total_pages, 3);
        assert_eq!(page.entries.len(), 20);
        assert_eq!(page.page_size, 20);
        assert!(
            page.entries
                .windows(2)
                .all(|pair| pair[0].time >= pair[1].time)
        );
        let last = summarize_with_logs(records, "all", false, 1, Some(999))
            .logs
            .unwrap();
        assert_eq!(last.page, 3);
        assert_eq!(last.entries.len(), 5);
        let tokens: u64 = page
            .entries
            .iter()
            .chain(last.entries.iter())
            .map(|e| e.tokens.total)
            .sum();
        assert_eq!(tokens, 25 * 11);
    }
    #[test]
    fn log_pagination_retains_prior_day_baseline_and_handles_empty_ranges() {
        let now = Local::now().timestamp_millis();
        let records = vec![
            record(now - 40 * 86400000, 100, 10, 100, 10),
            record(now, 150, 20, 50, 10),
        ];
        let page = summarize_with_logs(records, "7d", false, 1, Some(1))
            .logs
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.entries[0].tokens.total, 60);
        let empty = summarize_with_logs(Vec::new(), "today", false, 0, Some(0))
            .logs
            .unwrap();
        assert_eq!(empty.page, 1);
        assert_eq!(empty.total_pages, 1);
        assert!(empty.entries.is_empty());
    }
    #[test]
    fn cumulative_counts_are_differenced_and_duplicates_ignored() {
        let now = Local::now().timestamp_millis();
        let first = record(now - 100, 100, 20, 100, 20);
        let second = record(now, 150, 30, 50, 10);
        let summary = summarize(
            vec![first.clone(), first, second.clone(), second],
            "today",
            false,
            2,
        );
        assert_eq!(summary.tokens.total, 180);
        assert_eq!(summary.tokens.input, 150);
        assert_eq!(summary.requests, 2);
        assert_eq!(summary.tokens.cached, 25); // first record's last usage did not report cached input
    }
    #[test]
    fn missing_baseline_does_not_bill_historical_cumulative_on_current_date() {
        let summary = summarize(
            vec![record(
                Local::now().timestamp_millis(),
                100_000,
                10_000,
                50,
                5,
            )],
            "today",
            false,
            1,
        );
        assert_eq!(summary.tokens.total, 55);
        assert!(!summary.complete);
    }
    #[test]
    fn date_filters_keep_prior_cumulative_as_baseline() {
        let now = Local::now().timestamp_millis();
        let summary = summarize(
            vec![
                record(now - 40 * 86400000, 100, 10, 100, 10),
                record(now, 150, 20, 50, 10),
            ],
            "7d",
            false,
            1,
        );
        assert_eq!(summary.tokens.total, 60);
    }
    #[test]
    fn jsonl_reader_only_extracts_usage_and_session_metadata() {
        let path = std::env::temp_dir().join(format!(
            "assistant-usage-fixture-{}-{}.jsonl",
            std::process::id(),
            Local::now().timestamp_nanos_opt().unwrap()
        ));
        let text = format!(
            "{}\n{}\n{}\n{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"fixture-session"}}),
            serde_json::json!({"type":"turn_context","payload":{"model":"fixture-model"}}),
            serde_json::json!({"type":"response_item","payload":{"content":"PRIVATE-PROMPT-MUST-NOT-BE-INDEXED"}}),
            serde_json::json!({"timestamp":Local::now().to_rfc3339(),"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":3,"cached_input_tokens":4},"last_token_usage":{"input_tokens":10,"output_tokens":3,"cached_input_tokens":4}}}})
        );
        fs::write(&path, text).unwrap();
        let data = read_file(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(data.records.len(), 1);
        assert_eq!(data.records[0].session, "fixture-session");
        assert_eq!(data.records[0].model, "fixture-model");
        let summary = summarize(data.records, "all", data.partial, 1);
        assert_eq!(summary.tokens.total, 13);
        assert_eq!(summary.tokens.cached, 4);
        assert!(summary.complete);
        assert!(
            !serde_json::to_string(&summary)
                .unwrap()
                .contains("PRIVATE-PROMPT")
        );
    }
}
