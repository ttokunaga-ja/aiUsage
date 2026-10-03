# Windows実機検証

検証日：2026-10-03（日本時間）。既存のSSH接続からWindows 11 Pro x64へ接続し、WindowsネイティブのMSVCツールチェーンで検証した。WSLやmacOSのクロスコンパイルによる代用ではない。

## 環境と結果

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
