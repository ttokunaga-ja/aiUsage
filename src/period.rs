use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, TimeZone, Utc};
use clap::{ArgGroup, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "aiUsage",
    version,
    about = "Claude Code / Codexの保存ログを期間別・モデル別CSVへ出力"
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
#[command(group(ArgGroup::new("grouping").args(["day", "month", "year"]).multiple(false)))]
pub struct Args {
    /// 対象期間 (YYYY / YYYY-MM / YYYY-MM-DD)
    #[arg(conflicts_with_all = ["from", "to"])]
    pub range: Option<String>,
    /// 日別・モデル別に集計
    #[arg(long)]
    pub day: bool,
    /// 月別・モデル別に集計
    #[arg(long)]
    pub month: bool,
    /// 年別・モデル別に集計
    #[arg(long)]
    pub year: bool,
    /// 開始期間 (YYYY / YYYY-MM / YYYY-MM-DD)。省略時は保存ログの最初から
    #[arg(long)]
    pub from: Option<String>,
    /// 終了期間 (YYYY / YYYY-MM / YYYY-MM-DD)、指定期間の末尾を含む。省略時は現在まで
    #[arg(long)]
    pub to: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grouping {
    Total,
    Day,
    Month,
    Year,
}
impl Grouping {
    pub fn name(self) -> &'static str {
        match self {
            Self::Total => "total",
            Self::Day => "day",
            Self::Month => "month",
            Self::Year => "year",
        }
    }
}

pub struct Period {
    pub start: Option<DateTime<Utc>>,
    pub end: DateTime<Utc>,
    pub grouping: Grouping,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bucket {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub label: String,
}

fn jst() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).unwrap()
}
fn midnight(date: NaiveDate) -> DateTime<Utc> {
    jst()
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .unwrap()
        .with_timezone(&Utc)
}
fn next_month(date: NaiveDate) -> Result<NaiveDate> {
    let (year, month) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    NaiveDate::from_ymd_opt(year, month, 1).context("月の範囲が無効です")
}
fn bounds(value: &str) -> Result<(NaiveDate, NaiveDate)> {
    let format = match value.len() {
        4 => "%Y",
        7 => "%Y-%m",
        10 => "%Y-%m-%d",
        _ => bail!("期間はYYYY / YYYY-MM / YYYY-MM-DDで指定してください: {value}"),
    };
    let expanded = match value.len() {
        4 => format!("{value}-01-01"),
        7 => format!("{value}-01"),
        _ => value.to_owned(),
    };
    let start = NaiveDate::parse_from_str(&expanded, "%Y-%m-%d")
        .with_context(|| format!("期間が無効です: {value}"))?;
    if start.format(format).to_string() != value
        || !value.bytes().all(|b| b.is_ascii_digit() || b == b'-')
    {
        bail!("期間はYYYY / YYYY-MM / YYYY-MM-DDで指定してください: {value}");
    }
    let end = match value.len() {
        4 => NaiveDate::from_ymd_opt(start.year() + 1, 1, 1).context("年の範囲が無効です")?,
        7 => next_month(start)?,
        _ => start.succ_opt().context("終了日の翌日を計算できません")?,
    };
    Ok((start, end))
}

impl Period {
    pub fn from_args(args: &Args, now: DateTime<Utc>) -> Result<Self> {
        let (start, end) = if let Some(range) = &args.range {
            let (start, end) = bounds(range)?;
            (Some(start), Some(end))
        } else {
            let start = args.from.as_deref().map(bounds).transpose()?.map(|b| b.0);
            let end = args.to.as_deref().map(bounds).transpose()?.map(|b| b.1);
            if let (Some(start), Some(end)) = (start, end)
                && start >= end
            {
                bail!("--fromは--toの期間末尾以前にしてください");
            }
            (start, end)
        };
        let now_end = now
            .checked_add_signed(Duration::nanoseconds(1))
            .context("現在日時が範囲外です")?;
        Ok(Self {
            start: start.map(midnight),
            end: end.map(midnight).unwrap_or(now_end).min(now_end),
            grouping: if args.day {
                Grouping::Day
            } else if args.month {
                Grouping::Month
            } else if args.year {
                Grouping::Year
            } else {
                Grouping::Total
            },
        })
    }
    pub fn includes(&self, timestamp: DateTime<Utc>) -> bool {
        self.start.is_none_or(|start| timestamp >= start) && timestamp < self.end
    }
    pub fn bucket(&self, timestamp: DateTime<Utc>, earliest: DateTime<Utc>) -> Result<Bucket> {
        let selected_start = self.start.unwrap_or(earliest);
        if self.grouping == Grouping::Total {
            return Ok(Bucket {
                start: selected_start,
                end: self.end,
                label: "total".into(),
            });
        }
        let date = timestamp.with_timezone(&jst()).date_naive();
        let (start, end, label) = match self.grouping {
            Grouping::Day => (
                date,
                date.succ_opt().context("日付が範囲外です")?,
                date.format("%Y-%m-%d").to_string(),
            ),
            Grouping::Month => {
                let start = NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap();
                (start, next_month(start)?, date.format("%Y-%m").to_string())
            }
            Grouping::Year => (
                NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(date.year() + 1, 1, 1).context("年が範囲外です")?,
                date.format("%Y").to_string(),
            ),
            Grouping::Total => unreachable!(),
        };
        Ok(Bucket {
            start: midnight(start).max(selected_start),
            end: midnight(end).min(self.end),
            label,
        })
    }
}

pub fn csv_datetime(timestamp: DateTime<Utc>) -> String {
    timestamp
        .with_timezone(&jst())
        .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, false)
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
    fn precision_and_inclusive_end() {
        let p = period(&["2026-09", "--day"]);
        assert!(!p.includes("2026-08-31T14:59:59Z".parse().unwrap()));
        assert!(p.includes("2026-08-31T15:00:00Z".parse().unwrap()));
        assert!(!p.includes("2026-09-30T15:00:00Z".parse().unwrap()));
        assert_eq!(p.grouping, Grouping::Day);
        assert!(period(&["--to", "2026"]).includes("2026-10-03T10:00:00Z".parse().unwrap()));
        assert!(
            period(&["--from", "2026", "--to", "2026-09"])
                .includes("2026-09-30T14:59:59Z".parse().unwrap())
        );
        assert!(period(&["2024-02-29"]).includes("2024-02-28T15:00:00Z".parse().unwrap()));
    }
    #[test]
    fn bucket_bounds_clip_to_requested_range_and_now() {
        let p = period(&["--month", "--from", "2026-09-15", "--to", "2026-10"]);
        let first = p
            .bucket(
                "2026-09-17T00:00:00Z".parse().unwrap(),
                "2026-01-01T00:00:00Z".parse().unwrap(),
            )
            .unwrap();
        assert_eq!(csv_datetime(first.start), "2026-09-15T00:00:00+09:00");
        assert_eq!(csv_datetime(first.end), "2026-10-01T00:00:00+09:00");
        let last = p
            .bucket("2026-10-02T00:00:00Z".parse().unwrap(), first.start)
            .unwrap();
        assert_eq!(
            csv_datetime(last.end),
            "2026-10-03T19:00:00.000000001+09:00"
        );
        let p = period(&["--year"]);
        let stamp = "2026-09-17T00:00:00Z".parse().unwrap();
        assert_eq!(p.bucket(stamp, stamp).unwrap().start, stamp);
    }
    #[test]
    fn rejects_conflicts_removed_flag_and_invalid_dates() {
        for flags in [
            vec!["--all"],
            vec!["--day", "--month"],
            vec!["2026-09", "--from", "2026"],
            vec!["2026-09", "--to", "2026"],
        ] {
            assert!(Cli::try_parse_from(["aiUsage", "claude"].into_iter().chain(flags)).is_err());
        }
        for flags in [
            vec!["2026-02-30"],
            vec!["2026-9"],
            vec!["2026-09-1"],
            vec!["--from", "2026-10", "--to", "2026-09"],
        ] {
            let cli = Cli::try_parse_from(["aiUsage", "claude"].into_iter().chain(flags)).unwrap();
            let Command::Claude(args) = cli.command else {
                panic!("unexpected command")
            };
            assert!(Period::from_args(&args, Utc::now()).is_err());
        }
        assert_eq!(period(&[]).grouping, Grouping::Total);
        assert!(period(&[]).start.is_none());
    }
}
