use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, TimeZone, Utc};
use clap::{ArgGroup, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "aiUsage",
    version,
    about = "Claude Code / Codexの保存ログをモデル別CSVへ出力"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Claude Codeの保存ログを集計
    Claude(Args),
    /// Codexの保存ログを集計
    Chatgpt(Args),
    /// 最新の公開リリースへ実行ファイルを自動更新
    Update,
    /// 確認後に実行中の実行ファイルだけを削除
    Uninstall,
}

#[derive(Debug, clap::Args)]
#[command(group(ArgGroup::new("period").args(["day", "month", "year", "all"]).multiple(false)))]
pub struct Args {
    /// 日本時間の日付 (YYYY-MM-DD)
    #[arg(long, conflicts_with_all = ["from", "to"])]
    pub day: Option<String>,
    /// 日本時間の月 (YYYY-MM)
    #[arg(long, conflicts_with_all = ["from", "to"])]
    pub month: Option<String>,
    /// 日本時間の年 (YYYY)
    #[arg(long, conflicts_with_all = ["from", "to"])]
    pub year: Option<String>,
    /// 保存ログの全期間（期間指定なしの場合も同じ）
    #[arg(long, conflicts_with_all = ["from", "to"])]
    pub all: bool,
    /// 開始日 (YYYY-MM-DD)。省略時は保存ログの最初から
    #[arg(long)]
    pub from: Option<String>,
    /// 終了日 (YYYY-MM-DD)、当日を含む。省略時は現在まで
    #[arg(long)]
    pub to: Option<String>,
}

pub struct Period {
    pub start: Option<DateTime<Utc>>,
    pub end: DateTime<Utc>,
}

fn date(value: &str) -> Result<NaiveDate> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .with_context(|| format!("日付はYYYY-MM-DDで指定してください: {value}"))?;
    if date.format("%Y-%m-%d").to_string() != value {
        bail!("日付はYYYY-MM-DDで指定してください: {value}");
    }
    Ok(date)
}
fn midnight(date: NaiveDate) -> DateTime<Utc> {
    FixedOffset::east_opt(9 * 3600)
        .unwrap()
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .unwrap()
        .with_timezone(&Utc)
}
fn next(date: NaiveDate) -> Result<NaiveDate> {
    date.succ_opt().context("終了日の翌日を計算できません")
}

impl Period {
    pub fn from_args(args: &Args, now: DateTime<Utc>) -> Result<Self> {
        let (start, end) = if let Some(day) = &args.day {
            let d = date(day)?;
            (Some(d), Some(next(d)?))
        } else if let Some(month) = &args.month {
            if month.len() != 7 {
                bail!("月はYYYY-MMで指定してください");
            }
            let d = date(&format!("{month}-01"))?;
            let (y, m) = if d.month() == 12 {
                (d.year() + 1, 1)
            } else {
                (d.year(), d.month() + 1)
            };
            (
                Some(d),
                Some(NaiveDate::from_ymd_opt(y, m, 1).context("月の範囲が無効です")?),
            )
        } else if let Some(year) = &args.year {
            if year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
                bail!("年はYYYYで指定してください");
            }
            let y: i32 = year.parse()?;
            (
                Some(NaiveDate::from_ymd_opt(y, 1, 1).context("年が無効です")?),
                Some(NaiveDate::from_ymd_opt(y + 1, 1, 1).context("年が無効です")?),
            )
        } else {
            let start = args.from.as_deref().map(date).transpose()?;
            let last = args.to.as_deref().map(date).transpose()?;
            if let (Some(a), Some(b)) = (start, last)
                && a > b
            {
                bail!("--fromは--to以前の日付にしてください");
            }
            (start, last.map(next).transpose()?)
        };
        let now_end = now
            .checked_add_signed(Duration::nanoseconds(1))
            .context("現在日時が範囲外です")?;
        Ok(Self {
            start: start.map(midnight),
            end: end.map(midnight).unwrap_or(now_end).min(now_end),
        })
    }
    pub fn includes(&self, timestamp: DateTime<Utc>) -> bool {
        self.start.is_none_or(|start| timestamp >= start) && timestamp < self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn period(flags: &[&str]) -> Period {
        let cli = Cli::try_parse_from(
            ["aiUsage", "claude"]
                .into_iter()
                .chain(flags.iter().copied()),
        )
        .unwrap();
        let Command::Claude(args) = cli.command else {
            panic!("unexpected command")
        };
        Period::from_args(&args, "2026-10-03T10:00:00Z".parse().unwrap()).unwrap()
    }
    #[test]
    fn jst_boundaries_and_inclusive_end() {
        let p = period(&["--month", "2026-09"]);
        assert!(!p.includes("2026-08-31T14:59:59Z".parse().unwrap()));
        assert!(p.includes("2026-08-31T15:00:00Z".parse().unwrap()));
        assert!(!p.includes("2026-09-30T15:00:00Z".parse().unwrap()));
        let p = period(&["--from", "2026-09-01", "--to", "2026-09-30"]);
        assert!(p.includes("2026-09-30T14:59:59Z".parse().unwrap()));
    }
    #[test]
    fn open_ends_day_year_leap_day_and_default() {
        assert!(period(&["--to", "2026-09-30"]).start.is_none());
        assert!(
            period(&["--from", "2026-09-01"]).includes("2026-10-03T10:00:00Z".parse().unwrap())
        );
        assert!(period(&["--day", "2024-02-29"]).includes("2024-02-28T15:00:00Z".parse().unwrap()));
        assert!(period(&["--year", "2025"]).includes("2024-12-31T15:00:00Z".parse().unwrap()));
        assert!(period(&[]).start.is_none());
        assert!(!period(&["--all"]).includes("2026-10-04T00:00:00Z".parse().unwrap()));
    }
    #[test]
    fn rejects_conflicts_and_invalid_dates() {
        assert!(
            Cli::try_parse_from(["aiUsage", "claude", "--all", "--day", "2026-09-01"]).is_err()
        );
        for flags in [
            vec!["--day", "2026-02-30"],
            vec!["--month", "2026-9"],
            vec!["--from", "2026-10-01", "--to", "2026-09-01"],
        ] {
            let cli = Cli::try_parse_from(["aiUsage", "claude"].into_iter().chain(flags)).unwrap();
            let Command::Claude(args) = cli.command else {
                panic!("unexpected command")
            };
            assert!(Period::from_args(&args, Utc::now()).is_err());
        }
    }
}
