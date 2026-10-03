mod model;
mod parsers;
mod period;
mod pricing;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use clap::Parser;
use model::{Provider, Tokens};
use period::{Args, Period};
use pricing::Pricing;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

const OUTPUT: &str = "usage.csv";
const HEADER: [&str; 6] = [
    "区分",
    "通常入力",
    "キャッシュ読込",
    "キャッシュ書込",
    "出力",
    "API参考換算USD",
];

#[derive(Default)]
struct Row {
    tokens: Tokens,
    cost: u128,
    unpriced: bool,
}

fn roots(provider: Provider) -> Result<Vec<PathBuf>> {
    let home = dirs::home_dir().context("ホームディレクトリを取得できません")?;
    let env_path = |name: &str, fallback: PathBuf| -> Result<PathBuf> {
        match env::var_os(name) {
            Some(value) if value.is_empty() => bail!("{name}が空です"),
            Some(value) => Ok(PathBuf::from(value)),
            None => Ok(fallback),
        }
    };
    Ok(match provider {
        Provider::Claude => {
            vec![env_path("CLAUDE_CONFIG_DIR", home.join(".claude"))?.join("projects")]
        }
        Provider::Chatgpt => {
            let base = env_path("CODEX_HOME", home.join(".codex"))?;
            vec![base.join("sessions"), base.join("archived_sessions")]
        }
    })
}

fn fingerprint(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.file_type().is_file() {
                bail!("{}は通常ファイルではありません", path.display());
            }
            Ok(Some(Sha256::digest(fs::read(path)?).to_vec()))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn confirm(path: &Path) -> Result<Option<Vec<u8>>> {
    let original = fingerprint(path)?;
    if original.is_some() {
        eprint!(
            "警告: {}は既に存在します。上書きしますか？ [y/N]: ",
            path.display()
        );
        io::stderr().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes" | "はい") {
            bail!("出力を中止しました。既存ファイルは変更していません");
        }
    }
    Ok(original)
}

fn add_tokens(to: &mut Tokens, from: &Tokens) -> Result<()> {
    to.input = to
        .input
        .checked_add(from.input)
        .context("通常入力の合計が範囲外です")?;
    to.read = to
        .read
        .checked_add(from.read)
        .context("キャッシュ読込の合計が範囲外です")?;
    to.write = to
        .write
        .checked_add(from.write)
        .context("キャッシュ書込の合計が範囲外です")?;
    to.output = to
        .output
        .checked_add(from.output)
        .context("出力の合計が範囲外です")?;
    Ok(())
}

fn save(path: &Path, original: Option<Vec<u8>>, rows: &BTreeMap<String, Row>) -> Result<()> {
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().context("出力先が無効です")?)?;
    {
        let mut writer = csv::WriterBuilder::new()
            .terminator(csv::Terminator::CRLF)
            .from_writer(temporary.as_file_mut());
        writer.write_record(HEADER)?;
        for (model, row) in rows {
            writer.write_record([
                model.clone(),
                row.tokens.input.to_string(),
                row.tokens.read.to_string(),
                row.tokens.write.to_string(),
                row.tokens.output.to_string(),
                if row.unpriced {
                    String::new()
                } else {
                    pricing::usd(row.cost)
                },
            ])?;
        }
        writer.flush()?;
    }
    temporary.as_file().sync_all()?;
    if fingerprint(path)? != original {
        bail!(
            "集計中に{}が変更されました。上書きを中止します",
            path.display()
        );
    }
    if original.is_some() {
        temporary.persist(path).map_err(|e| e.error)?;
    } else {
        temporary.persist_noclobber(path).map_err(|e| e.error)?;
    }
    Ok(())
}

fn run() -> Result<()> {
    let args = Args::parse();
    let provider = Provider::from(args.source);
    let period = Period::from_args(&args, Utc::now())?;
    let paths = roots(provider)?;
    let output = env::current_dir()?.join(OUTPUT);
    let original = confirm(&output)?;
    if !paths.iter().any(|p| p.exists()) {
        bail!(
            "保存ログが見つかりません: {}",
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    eprintln!("保存ログを読み取っています…");
    let scan = parsers::scan(provider, &paths)?;
    if scan.unstable > 0 {
        bail!(
            "読み取り中に{}件のログが変更されました。ログの更新が止まってから再実行してください",
            scan.unstable
        );
    }
    if scan.files == 0 {
        bail!("JSONL形式の保存ログが見つかりません");
    }
    let pricing = Pricing::new();
    let mut rows = BTreeMap::<String, Row>::new();
    for event in scan.events.iter().filter(|e| period.includes(e.timestamp)) {
        let row = rows.entry(event.model.clone()).or_default();
        add_tokens(&mut row.tokens, &event.tokens)?;
        match pricing.cost(provider, event) {
            Some(cost) => {
                row.cost = row
                    .cost
                    .checked_add(cost)
                    .context("API参考換算が範囲外です")?
            }
            None => row.unpriced = true,
        }
    }
    save(&output, original, &rows)?;
    if scan.malformed > 0 || scan.excluded > 0 {
        eprintln!(
            "警告: 解析できない行{}件、利用量を確定できず除外した記録{}件",
            scan.malformed, scan.excluded
        );
    }
    let unpriced: Vec<_> = rows
        .iter()
        .filter(|(_, r)| r.unpriced)
        .map(|(m, _)| m.as_str())
        .collect();
    if !unpriced.is_empty() {
        eprintln!(
            "警告: 単価・キャッシュTTLなどを確定できないためAPI参考換算USDを空欄にしたモデル: {}",
            unpriced.join(", ")
        );
    }
    if rows.is_empty() {
        eprintln!("指定期間の利用記録はありません。CSVはヘッダーのみです");
    }
    println!("{}", output.display());
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("エラー: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_fixed_columns_without_totals_and_refuse_changed_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(OUTPUT);
        let mut rows = BTreeMap::new();
        rows.insert(
            "model".into(),
            Row {
                tokens: Tokens {
                    input: 123,
                    read: 456,
                    write: 78,
                    output: 9,
                    ..Tokens::default()
                },
                cost: 100_000_000_000_000,
                unpriced: false,
            },
        );
        save(&path, None, &rows).unwrap();
        let mut reader = csv::Reader::from_path(&path).unwrap();
        assert_eq!(reader.headers().unwrap().iter().collect::<Vec<_>>(), HEADER);
        let records: Vec<_> = reader.records().map(Result::unwrap).collect();
        assert_eq!(records.len(), 1);
        assert_eq!(&records[0][1], "123");
        let before = fingerprint(&path).unwrap();
        fs::write(&path, "changed").unwrap();
        assert!(save(&path, before, &rows).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "changed");
    }
}
