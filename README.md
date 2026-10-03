# aiUsage

Claude Code / Codexのローカル保存ログを読み、モデル別の利用量とAPI参考換算を`usage.csv`に出力するRust製CLIです。実行ファイルには単価表を同梱し、実行時にPython・Rust・APIキーは不要です。

## 使い方

```sh
aiUsage claude --day 2026-09-15
aiUsage claude --month 2026-09
aiUsage chatgpt --year 2026
aiUsage chatgpt --from 2026-09-01 --to 2026-09-30
aiUsage chatgpt --from 2026-09-01
aiUsage chatgpt --to 2026-09-30
aiUsage claude --all
```

- `claude`はClaude Code、`chatgpt`はCodexのローカルログを対象とします。
- 日付は日本時間です。`--to`は指定した日を含みます。
- `--from`のみなら指定日から現在まで、`--to`のみなら保存ログの最初から指定日まで取得します。
- `--all`または期間指定なしなら、保存ログの全期間を現在まで取得します。
- `--day YYYY-MM-DD`・`--month YYYY-MM`・`--year YYYY`・`--all`は併用できません。`--from`/`--to`との併用もできません。

常に**コマンドを実行したフォルダ**の`usage.csv`へ保存します。モデル別の指定、出力先・ファイル名の指定はありません。同名のファイルがあれば上書きの警告を表示し、`y`/`yes`/`はい`で許可した場合のみ置き換えます。Enterのみ、その他の入力、入力終了では中止します。

出力はUTF-8のCSVで、次の6列に固定します。区分はログに記録されたモデル名です。指定期間につきモデルごとに1行で、合計行は出しません。整数は桁区切りなし、USDは丸めずに出力します。

```csv
区分,通常入力,キャッシュ読込,キャッシュ書込,出力,API参考換算USD
gpt-6.1-sol,20,80,0,10,0.000148
```

`--all`/`--year`でも同じ形式です。月別・日別の行には分割せず、指定期間全体をモデルごとに出力します。期間に記録がなければヘッダーだけ保存します。

## 読み取る場所

| 指定 | 既定の保存ログ |
| --- | --- |
| `claude` | `~/.claude/projects/**/*.jsonl` |
| `chatgpt` | `~/.codex/sessions/**/*.jsonl` と `~/.codex/archived_sessions/**/*.jsonl` |

`CLAUDE_CONFIG_DIR`または`CODEX_HOME`を設定している場合はそのフォルダを使用します。別のログ保存先を使う場合も、この環境変数で切り替えます。元のログ・設定・DBは書き換えません。会話本文はCSVに含めません。

## 集計と単価

Claudeは応答usageをリクエストID・メッセージID・モデルで重複排除し、ストリーミングの同一応答更新をまとめます。5分/1時間のキャッシュ書き込みをそれぞれの単価で換算します。

Codexは実行ID単位の累積カウンター差分を、直近リクエストusageと突合して集計します。期間の開始前の記録も読み、差分の起点として使います。同一実行のコピー・同一累積値の反復は加算しません。フォークの元実行IDと親の記録が残り、時刻・モデル・provider・カウンターが一致するコピー記録も除外します。別実行の親子やサブエージェントの独立した利用は加算します。入力からキャッシュを引いた値が通常入力です。推論トークンは出力に含まれているため再加算しません。

DBの会話累積値、スクリーンショット、キャッシュ比率の推計は使用しません。消えたログの復元は行いません。曖昧な累積値・不正な記録は除外して端末に件数を表示します。各ファイルは読み取り開始時のサイズまで取得し、その後の追記は次回に含めます。読み取り中の切り詰め・置き換え・同サイズの書き換えを検知した場合は保存を中止します。

2026-10-03確認のStandard API単価を全期間に適用します。過去の請求額・サブスク料金ではなく、トークン部分の参考換算です。モデル不明、単価未収録、必要なキャッシュTTLが欠けている場合は、トークン数は出力し、そのモデルのAPI参考換算USDを空欄にして端末に警告します。一部だけの換算額をモデル全体の金額として出しません。

- [OpenAI公式料金表](https://developers.openai.com/api/docs/pricing)
- [Claude公式料金表](https://platform.claude.com/docs/en/about-claude/pricing)
- `data/openai-pricing.json` / `data/claude-pricing.json`をビルド時に埋め込みます。単価更新はソースの単価表を更新して再ビルドします。

大量ログの初回走査には時間がかかります。メモリにはusageのメタデータを保持し、会話本文は保持しません。

## ビルド・検証

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

macOS/Linuxの実行ファイルは`target/release/aiUsage`、Windowsは`target/release/aiUsage.exe`です。このファイルをコピーして使えます。

```sh
mkdir -p ~/.local/bin
cp target/release/aiUsage ~/.local/bin/aiUsage
```

`~/.local/bin`がPATHにあれば、任意のフォルダで`aiUsage`を呼び出せます。

Windows（PowerShell）では、コピーした実行ファイルを次のように使います。

```powershell
.\aiUsage.exe --version
.\aiUsage.exe claude --month 2026-09
.\aiUsage.exe chatgpt --from 2026-09-01
```

Windows MSVC版はCランタイムを静的リンクし、追加のVC++ランタイムのインストールを必要としない設定です。

Windows上のソース検証は`cmd.exe /c scripts\verify-windows.cmd`で再実行できます。配布版と同じMSVCツールチェーンで、整形・Clippy・テスト・リリースビルド・バージョン表示を検査します。Windows 11 Pro実機での結果は[Windows検証記録](docs/windows-verification.md)に記載しています。GitHub Actionsの通常検査もLinux・macOS・Windowsで実行する構成です。

## オプションとバージョン・更新

`-h` / `--help`で使い方、`-V` / `--version`で現在の版を表示します。バージョン表示は通信やCSV出力を行いません。

1文字の短縮オプションは`-h`、長い名前のオプションは`--month`のように記述します。期間オプションは現在、長い名前のみです。`-month`や`-update`の独自形式は採用していません。

`aiUsage update`で最新の公開リリースを取得し、実行ファイルを自動で置き換えます。単価表も実行ファイルに含まれるため、一緒に更新されます。

```sh
aiUsage update
```

GitHubの`ttokunaga-ja/aiUsage`の最新正式リリースから、macOSまたはWindows用のファイルと`SHA256SUMS`を取得します。SHA-256と新しい実行ファイルの`--version`を確認してから置き換えます。最新版の場合はそのまま終了します。通常の計測・`--version`では更新確認を行いません。

通信にはOSの`curl`を使用し、実行ファイルのあるフォルダへの書き込み権限が必要です。Windowsでは実行中の旧exeを退避して置き換え、失敗時には元へ戻します。残った退避ファイルの場所は表示します。CSVや保存ログは変更しません。

配布用の実行ファイルは[GitHub Releases](https://github.com/ttokunaga-ja/aiUsage/releases)から取得できます。公開するタグはCargoの版と一致する`vX.Y.Z`にします。リリースワークフローがOS別の実行ファイルと`SHA256SUMS`を作成します。

参考：[clapのshort/longオプション](https://docs.rs/clap/latest/clap/struct.Arg.html)、[GitHub Releases API](https://docs.github.com/en/rest/releases/releases#get-the-latest-release)。
