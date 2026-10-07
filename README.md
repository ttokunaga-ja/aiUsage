# aiUsage

Claude Code / Codexのローカル保存ログを読み、モデル別の利用量とAPI参考換算を`usage.csv`に出力するRust製CLIです。実行ファイルには単価表を同梱し、実行時にPython・Rust・APIキーは不要です。

## 必要なもの

- macOS（Apple Silicon / Intel）またはWindows x64
- 集計したいClaude Code / Codexの保存ログ
- インストール・更新時のインターネット接続

配布版を使う場合はRust・Python・APIキーは不要です。管理者権限も必要ありません。通常の集計はローカルで行い、保存ログを外部へ送信しません。Linuxでは[ソースからビルド](#ビルド検証)して使えます。

## はじめかた

### 1. インストールする

OSに合うコマンドを1回実行します。最新の正式リリースをダウンロードし、SHA-256とバージョンを確認してからインストールします。Rust・Pythonのインストールや初期設定は不要です。

#### macOS（ターミナル）

```sh
curl -fsSL https://raw.githubusercontent.com/ttokunaga-ja/aiUsage/main/install.sh | sh
```

Apple Silicon・Intel共通です。`~/.local/bin/aiUsage`へインストールし、使用しているシェルの設定ファイル（zshなら`~/.zshrc`）へPATHの設定を追加します。**新しいターミナルを開く**と`aiUsage`を使えます。

#### Windows x64（PowerShell）

```powershell
irm https://raw.githubusercontent.com/ttokunaga-ja/aiUsage/main/install.ps1 | iex
```

`%USERPROFILE%\.local\bin\aiUsage.exe`へインストールし、ユーザーPATHと現在のPowerShellのPATHへ追加します。そのまま`aiUsage`を使えます。Cランタイムは静的リンクしているため、追加のVC++ランタイムは不要です。

実行ファイルの検証や実行に失敗した場合は、既存の実行ファイルを置き換えません。

実行ファイルと`SHA256SUMS`を手動で取得する場合は、[最新のReleases](https://github.com/ttokunaga-ja/aiUsage/releases/latest)を利用できます。

### 2. 動作確認して集計する

macOS・Windowsともに同じコマンドです。

```sh
aiUsage --version
aiUsage --help
aiUsage claude 2026-09 --month
```

初期設定やログイン操作は不要です。集計結果の`usage.csv`は**コマンドを実行したフォルダ**に保存されます。CSVを保存したいフォルダへ移動してから実行してください。

## 使い方

対象期間と集計単位を別々に指定します。期間は位置引数または`--from`・`--to`、集計単位は`--day`・`--month`・`--year`です。

```text
aiUsage <claude|chatgpt> [対象期間] [--day|--month|--year] [--from 開始] [--to 終了]
```

| コマンド | 対象期間 | 出力単位 |
| --- | --- | --- |
| `aiUsage claude` | 保存ログの全期間 | 全期間・モデル別 |
| `aiUsage claude --month` | 保存ログの全期間 | 月別・モデル別 |
| `aiUsage claude --day` | 保存ログの全期間 | 日別・モデル別 |
| `aiUsage chatgpt --year` | 保存ログの全期間 | 年別・モデル別 |
| `aiUsage claude 2026-09 --month` | 2026年9月 | 月別・モデル別 |
| `aiUsage claude 2026-09 --day` | 2026年9月 | 日別・モデル別 |
| `aiUsage chatgpt 2026 --month` | 2026年 | 月別・モデル別 |
| `aiUsage chatgpt 2026-09` | 2026年9月 | 対象期間・モデル別 |
| `aiUsage chatgpt --month --from 2026-03 --to 2026-09` | 3〜9月 | 月別・モデル別 |

- `claude`はClaude Code、`chatgpt`はCodexのローカルログを対象とします。
- 対象期間・`--from`・`--to`は`YYYY`・`YYYY-MM`・`YYYY-MM-DD`で指定できます。日付と日・月・年の区切りは日本時間です。
- `--from 2026-09`は9月1日から、`--to 2026-09`は9月末までを表します。終了に指定した日・月・年全体を含みます。
- `--from`のみなら指定期間の先頭から現在まで、`--to`のみなら保存ログの最初から指定期間の末尾まで取得します。未来の期間末尾は実行時点で打ち切ります。
- 位置引数の対象期間と`--from`・`--to`は併用できません。集計単位は1つだけ指定できます。
- 集計単位を省略すると、対象期間を区切らずモデル別に集計します。期間も省略すると保存ログの全期間になります。`--all`はありません。

### CSV形式

常に**コマンドを実行したフォルダ**の`usage.csv`へ保存します。モデル別の指定、出力先・ファイル名の指定はありません。同名のファイルがあれば上書きの警告を表示し、`y`/`yes`/`はい`で許可した場合のみ置き換えます。Enterのみ、その他の入力、入力終了では中止します。

1行は「サービス × 集計単位 × 期間 × モデル」の集計結果です。全ての集計単位で、UTF-8・同じ11列を使います。

```csv
サービス,集計単位,期間,集計開始,集計終了,区分,通常入力,キャッシュ読込,キャッシュ書込,出力,API参考換算USD
```

| 列 | 内容 |
| --- | --- |
| サービス | `claude`または`chatgpt` |
| 集計単位 | `day`・`month`・`year`・`total` |
| 期間 | 日別`YYYY-MM-DD`、月別`YYYY-MM`、年別`YYYY`、分割なし`total` |
| 集計開始 | その行の対象範囲の開始日時（含む） |
| 集計終了 | その行の対象範囲の終了日時（含まない） |
| 区分 | ログに記録されたモデル名 |
| 通常入力・キャッシュ読込・キャッシュ書込・出力 | トークン数 |
| API参考換算USD | 同梱した単価での参考換算額。換算できない場合は空欄 |

日時は`2026-09-01T00:00:00+09:00`のようなタイムゾーン付きISO 8601形式です。開始を省略した場合は、有効な保存ログの最初の記録を開始に使います。日・月・年の範囲は指定した対象期間で切り取ります。9月15日から月別集計すれば、9月の行の開始は9月15日です。現在の月は実行時点で打ち切り、月途中の集計であることもCSVから判別できます。

合計行・コメント行・空の期間やモデルの行は出しません。対象期間に利用記録がなければヘッダーだけ保存します。行は期間順、同じ期間ではモデル名順です。整数は桁区切りなし、USDは通貨記号なしの10進数で、表示のための丸めはしません。確定したゼロは`0`、換算できない金額は空欄です。CSVのカンマ・引用符・改行は標準のCSVエスケープで扱います。

月別・モデル別の合計やキャッシュ比率は、CSVを表計算ソフトやプログラムで加工できます。複数CSVを結合する際は、再出力した同じ結果や期間の重なる結果を重ねて加算しないよう、サービス・集計単位・期間・集計開始・集計終了・区分を確認してください。

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

ソースから使う場合はRustとGitを用意し、リポジトリを取得してビルドします。

```sh
git clone https://github.com/ttokunaga-ja/aiUsage.git
cd aiUsage
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

macOS/Linuxの実行ファイルは`target/release/aiUsage`、Windowsは`target/release/aiUsage.exe`です。このファイルをコピーして使えます。

ユーザー用のフォルダへコピーする場合：

macOS/Linux：

```sh
mkdir -p "$HOME/.local/bin"
install -m 755 target/release/aiUsage "$HOME/.local/bin/aiUsage"
```

Windows（PowerShell）：

```powershell
$bin = Join-Path $env:USERPROFILE '.local\bin'
New-Item -ItemType Directory -Path $bin -Force | Out-Null
Copy-Item -LiteralPath 'target\release\aiUsage.exe' -Destination (Join-Path $bin 'aiUsage.exe') -Force
```

ソースからコピーした場合は、コピー先をシェルまたはWindowsのユーザーPATHへ追加してください。

インストーラーの隔離検証では、`BIN_DIR`に絶対パスのテスト用フォルダを指定し、`AI_USAGE_INSTALL_NO_PATH=1`でシェル設定・ユーザーPATHの変更を省略できます。この変数は検証用です。通常のインストールでは指定せず、PATHを自動設定します。

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

タグを公開したときに新しいリリースが作られます。`main`に入っただけの変更は`aiUsage update`の対象になりません。Linuxでは自動更新に対応していないため、新しいソースを取得して再ビルドします。

## アンインストール

```sh
aiUsage uninstall
```

実際に実行中の実行ファイルの絶対パスを表示し、`[y/N]`で確認します。`y`・`yes`・`はい`だけが削除を許可します。Enter・EOF・それ以外の入力では変更しません。確認を省略するオプションはありません。CSV・保存ログ・設定・PATH・共有binフォルダは残ります。

Windowsでは実行中のexeを同じフォルダの一意な退避ファイルへ移し、OS標準PowerShellで終了後の削除を予約します。表示するJSON記録の`status`が`deleted`なら削除完了、`scheduled`なら待機中、`failed`なら失敗です。削除完了を確認するまで記録を保管してください。失敗時はプロセス終了後に表示された退避ファイルだけを手動削除してください。元のパスに新しく置かれた実行ファイルは削除しません。ヘルパーの準備に失敗した場合は復元し、別のファイルが元の場所にある場合は上書きせず退避ファイルの場所を表示します。

手動で削除する場合：

macOS：

```sh
rm "$HOME/.local/bin/aiUsage"
```

Windows（PowerShell）：

```powershell
Remove-Item -LiteralPath (Join-Path $env:USERPROFILE '.local\bin\aiUsage.exe')
```

実行ファイルを削除すればアンインストールできます。出力済みのCSVとClaude Code / Codexの保存ログは残ります。`.local/bin`は他のツールも使うため、フォルダやPATHの項目をまとめて削除する必要はありません。

## ライセンス

[MIT](LICENSE)
