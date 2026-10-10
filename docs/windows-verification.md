# Windows実機検証

## 0.2.1の価格表検証

2026-10-10（日本時間）、Windows 11 x64実機で`scripts/verify-windows.cmd`を実行し、整形・Clippy・28件の単体テスト・18件のCLIテスト・MSVCリリースビルド・バージョン表示が成功した。Daybreak Blueのキャッシュ別・長文単価、Claude新モデル、Haiku 5.5の100,000／100,001トークン境界、両TTLの書き込みを含む入力長、日付付きモデルID、未知TTLの空欄を検査した。CLIテストでDaybreakとHaikuのCSV換算額と未登録モデル警告の解消も確認した。

macOS・Linuxでも整形・Clippy・44件のテスト・リリースビルドが成功した。macOSは両CPU向けUniversalバイナリの署名とApple Silicon上のCSV出力を確認した。生ログはローカルの`dist/validation/v0.2.1/`に保存した。Intel Mac実機の起動は未検証。

## 0.2.0の期間別CSV検証

2026-10-07（日本時間）、SSH経由でWindows 11 x64実機の一時フォルダへソースをコピーし、`scripts/verify-windows.cmd`を実行した。MSVC版Rust 1.97.1で整形・Clippy・25件の単体テスト・16件のCLIテスト・リリースビルド・`aiUsage 0.2.0`の表示がすべて成功した。

月別・日別・年別の一括集計、年・月・日精度の期間指定、日本時間の境界、部分期間の開始・終了、11列CSV、モデル順、CSVエスケープ、未知の単価の空欄、上書きの拒否と許可を確認した。分割後の各トークン数とAPI参考換算の合計が、同期間の分割なし集計と一致することも検査した。コピーexeの独立動作、Windows固有の更新・アンインストールの回帰テストも成功した。

macOSとLinuxでも整形・Clippy・39件のテスト・リリースビルドが通過した。macOSはApple SiliconとIntel向けをビルドしてUniversalバイナリを作成し、署名検査とApple Silicon上の起動・月別/日別CSV出力を確認した。Intel実機での起動は今回未検証。生ログはローカルの`dist/validation/v0.2.0/`に保存し、Gitへ含めていない。

以下は旧版で実施した検証の記録。

検証日：2026-10-03（日本時間）。既存のSSH接続からWindows 11 Pro x64へ接続し、WindowsネイティブのMSVCツールチェーンで検証した。WSLやmacOSのクロスコンパイルによる代用ではない。

## 0.1.0の環境と結果

| 項目 | 結果 |
| --- | --- |
| OS | Microsoft Windows 11 Pro / AMD64 |
| ツールチェーン | stable-x86_64-pc-windows-msvc |
| rustc | 1.97.1 |
| 製品バージョン | aiUsage 0.1.0 |
| `cargo fmt --check` | 成功 |
| `cargo clippy --locked --all-targets -- -D warnings` | 成功 |
| `cargo test --locked` | 20件成功（ユニット15・CLI5） |
| `cargo build --release --locked` | 成功 |
| コピーしたリリースexeの`--version` | 成功 |
| VC++ランタイム | 静的リンク。追加のVC++ランタイムDLLへの依存がないことをPEインポート表で確認 |
| 日本語・空白のあるフォルダからの実行 | 成功 |
| ClaudeのWindows保存ログ13ファイル、約78MBからのCSV出力 | 成功、モデル別3行 |
| CSVの固定ファイル名・実行フォルダ・UTF-8の6列ヘッダー | 確認済み |
| `n`による上書き拒否 | 終了コード非0、元CSVのSHA-256が不変 |
| `y`による上書き許可 | 終了コード0、再生成成功 |

CLIテストは日・月・年・全期間、片側だけの期間指定、日本時間の境界、引数の矛盾、上書き拒否・許可・EOF、Codexのキャッシュ区分、コピーしたexeの独立動作を検査する。CodexはこのWindowsアカウントに保存ログが見つからなかったため、Windows上ではテスト用ログによる検証である。Claudeは同アカウントの実ログも読み取った。

実ログの走査では利用量を確定できない16記録が除外された。CSV生成・上書き確認の動作検証に使用しており、アカウント全体の請求総額との一致を検証したものではない。

WindowsのPowerShellファイル実行はローカルポリシーで制限されていた。ポリシーは変更せず、直接のCargoコマンドと`cmd.exe`の検証用バッチで実行した。元のログ・設定・DBは変更していない。検証用のコピー・CSV・ビルド成果物は一意な一時フォルダ内に作成した。

## 再実行

MSVC版Rustとビルド環境があるWindowsで、リポジトリのルートから実行する。

```powershell
cmd.exe /c scripts\verify-windows.cmd
```

ツールチェーン名を指定する場合：

```powershell
cmd.exe /c scripts\verify-windows.cmd stable-x86_64-pc-windows-msvc
```

検証した配布用exeのSHA-256：

```text
447e71f0896731199f9fc3cca95d3f308611dc2c9e2af108481ca458016c6372
```

同じexeをSSHでmacOSへ回収し、SHA-256が実機検証時の値と一致することを確認した。生のビルド出力と実機レポートはローカルの`dist/validation/windows/`に保管し、Gitには含めていない。

初回のMSVCビルドでは`VCRUNTIME140.dll`への依存があったため、`.cargo/config.toml`でWindows MSVC向けに`crt-static`を指定した。その設定で20件のテストと実ログによるコピーexeの動作を再検証した。最終exeのインポート表はWindows標準DLLのみで、`VCRUNTIME140.dll`と`MSVCP`系DLLは含まれない。

参考：[Rust公式・Cランタイムの静的リンク](https://doc.rust-lang.org/stable/reference/linkage.html#static-and-dynamic-c-runtimes)。

## 0.1.1の自動更新検証

同じWindows実機で`aiUsage update`の実装を追加した版を再検証した。整形、Clippy、28件のテスト（ユニット22・CLI6）、MSVCのリリースビルドが通過した。

更新のテストでは、不正なタグ・ハッシュ・候補バージョンの拒否、同じ版の更新省略、更新ロックの排他と解除、一時ファイルの後始末、置き換え失敗時の復元、復元失敗時の旧版保持を確認した。コピーしたテスト用exeを実際に起動した状態で退避・置き換えを行うテストもWindows上で成功した。ダウンロード部分はテスト用データを注入しており、製品には配布元を差し替えるオプションや環境変数は設けていない。

製品exeの`--version`と`update --help`も成功した。公開前の検証時点では公式リリースAPIが404を返したため、製品の`update`は終了コード1で終了した。その前後でexeのSHA-256が変わらず、更新ロックが残らないことを確認した。公開済み新バージョンからの一連の更新は、リリース公開後の確認が必要になる。

0.1.1のWindows配布用exeのSHA-256：

```text
33b8c7380bd459072f51613ce21788b0c5be8e5c25f20aa50481a1a2c60633b2
```

SSH経由で回収したexeのハッシュも一致した。検証出力は`dist/validation/windows/updater-build.log`と`updater-runtime.log`に保存している。macOSでは27件のテスト、両CPU向けのビルド、Universalバイナリの署名検査、公開リリースが取得できない場合の旧版保持・ロック解除を確認した。Intel向けのビルドは成功しているが、Intel実機での起動は今回検証していない。
