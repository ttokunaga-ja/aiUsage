//! Standalone executable-only uninstall; no application-specific dependencies.
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, Write},
    path::Path,
};

fn fingerprint(path: &Path) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        bail!("削除対象が通常ファイルではありません: {}", path.display());
    }
    Ok(Sha256::digest(fs::read(path)?).to_vec())
}

#[cfg(unix)]
fn identity(path: &Path) -> Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)?;
    Ok((metadata.dev(), metadata.ino()))
}

/// Prompt for the canonical running executable and remove only that executable.
pub fn run(app_name: &str, preservation_message: &str) -> Result<()> {
    let executable =
        fs::canonicalize(env::current_exe()?).context("実行中の実行ファイルを特定できません")?;
    let original = fingerprint(&executable)?;
    #[cfg(unix)]
    let original_identity = identity(&executable)?;
    eprintln!("{app_name} の削除対象: {}", executable.display());
    eprintln!("{preservation_message}");
    eprint!("この実行ファイルを削除しますか？ [y/N]: ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes" | "はい") {
        eprintln!("キャンセルしました。変更していません。");
        return Ok(());
    }
    if fingerprint(&executable)? != original {
        bail!("確認中に実行ファイルが変更されました。削除を中止しました");
    }
    #[cfg(unix)]
    if identity(&executable)? != original_identity {
        bail!("確認中に実行ファイルが置き換えられました。削除を中止しました");
    }
    #[cfg(windows)]
    return windows::schedule(&executable, &original);
    #[cfg(not(windows))]
    {
        fs::remove_file(&executable).context("実行ファイルを削除できません")?;
        eprintln!("実行ファイルを削除しました: {}", executable.display());
        Ok(())
    }
}

// A failed restore must not drop the helper's persistent diagnostic receipt.
#[cfg(any(windows, test))]
fn rollback_with_receipt(
    receipt_directory: tempfile::TempDir,
    restore: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if let Err(error) = restore() {
        let kept = receipt_directory.keep();
        let receipt = kept.join("receipt.json");
        if !receipt.exists() {
            // Launch failures have no helper receipt yet. Preserve a static receipt
            // and put the detailed restore error in the returned diagnostic.
            if let Err(write_error) = fs::write(
                &receipt,
                br#"{"status":"failed","detail":"Helper setup failed and executable restore failed"}"#,
            ) {
                bail!("{error:#}。記録を書き込めません: {write_error}。記録の保存先: {}", receipt.display());
            }
        }
        bail!("{error:#}。結果の記録: {}", receipt.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_restore_preserves_existing_helper_receipt() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("receipt.json");
        let original = br#"{"status":"failed","detail":"helper failure"}"#;
        fs::write(&path, original).unwrap();
        let error = rollback_with_receipt(directory, || {
            bail!("元の場所に別のファイルがあるため上書きしません")
        })
        .unwrap_err();
        assert!(error.to_string().contains(&path.display().to_string()));
        assert!(error.to_string().contains("上書きしません"));
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn failed_launch_and_restore_create_a_persistent_failure_receipt() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("receipt.json");
        assert!(rollback_with_receipt(directory, || bail!("staged bytes changed")).is_err());
        assert!(fs::read_to_string(&path).unwrap().contains("failed"));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn successful_restore_discards_cancelled_helper_receipt() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_path_buf();
        fs::write(path.join("receipt.json"), b"cancelled").unwrap();
        rollback_with_receipt(directory, || Ok(())).unwrap();
        assert!(!path.exists());
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{
        os::windows::process::CommandExt,
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    // All variable data travels through child-only environment variables, never source.
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$staged = $env:EXEC_UNINSTALL_STAGED
$receipt = $env:EXEC_UNINSTALL_RECEIPT
$ready = $env:EXEC_UNINSTALL_READY
$expected = $env:EXEC_UNINSTALL_HASH
$parentId = [int]$env:EXEC_UNINSTALL_PID
function Write-Receipt([string]$status, [string]$detail) {
    $record = @{ status = $status; detail = $detail; staged_path = $staged; original_path = $env:EXEC_UNINSTALL_ORIGINAL; parent_pid = $parentId; expected_sha256 = $expected; utc = [DateTime]::UtcNow.ToString('o') }
    $nextReceipt = $receipt + '.next'
    [IO.File]::WriteAllText($nextReceipt, ($record | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false))
    if ([IO.File]::Exists($receipt)) {
        # Windows PowerShell 5.1 coerces a null string argument into an invalid
        # empty backup path. An explicit private backup keeps replacement atomic.
        $previousReceipt = $receipt + '.previous'
        [IO.File]::Replace($nextReceipt, $receipt, $previousReceipt)
        [IO.File]::Delete($previousReceipt)
    }
    else { [IO.File]::Move($nextReceipt, $receipt) }
}
try {
    $parent = [Diagnostics.Process]::GetProcessById($parentId)
    $null = $parent.Handle
    if ((Get-FileHash -LiteralPath $staged -Algorithm SHA256).Hash -ne $expected) { throw 'Staged executable fingerprint changed before scheduling' }
    Write-Receipt 'scheduled' 'Waiting for the uninstall process to exit'
    [IO.File]::WriteAllText($ready, 'ready')
    if (-not $parent.WaitForExit(120000)) { throw 'Parent did not exit within 120 seconds; staged executable retained' }
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ($true) {
        if ((Get-FileHash -LiteralPath $staged -Algorithm SHA256).Hash -ne $expected) { throw 'Staged executable fingerprint changed; retained' }
        try { Remove-Item -LiteralPath $staged -Force -ErrorAction Stop; break }
        catch { if ([DateTime]::UtcNow -ge $deadline) { throw }; Start-Sleep -Milliseconds 100 }
    }
    Write-Receipt 'deleted' 'Staged executable deleted after parent exit'
} catch {
    Write-Receipt 'failed' $_.Exception.Message
    exit 1
}
"#;

    fn rollback(executable: &Path, staged: &Path, hash: &[u8]) -> Result<()> {
        if fingerprint(staged).with_context(|| {
            format!(
                "退避ファイルを確認できません。残存ファイル: {}",
                staged.display()
            )
        })? != hash
        {
            bail!(
                "退避ファイルが変更されたため復元を中止しました。残存ファイル: {}",
                staged.display()
            );
        }
        match fs::symlink_metadata(executable) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            _ => bail!(
                "元の場所に別のファイルがあるため上書きしません。残存ファイル: {}",
                staged.display()
            ),
        }
        fs::rename(staged, executable)
            .with_context(|| format!("復元できません。残存ファイル: {}", staged.display()))
    }

    fn stop(child: &mut Child) -> Result<()> {
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        child.wait()?;
        Ok(())
    }

    pub fn schedule(executable: &Path, hash: &[u8]) -> Result<()> {
        let directory = executable
            .parent()
            .context("実行ファイルの親フォルダがありません")?;
        let reserved = tempfile::Builder::new()
            .prefix(".uninstall-")
            .suffix(".pending-delete.exe")
            .tempfile_in(directory)?;
        let staged = reserved.path().to_path_buf();
        reserved.close()?;
        let receipt_directory = tempfile::Builder::new()
            .prefix("executable-uninstall-")
            .tempdir()?;
        let receipt = receipt_directory.path().join("receipt.json");
        let ready = receipt_directory.path().join("ready");
        if fingerprint(executable)? != hash {
            bail!("退避前に実行ファイルが変更されました");
        }
        fs::rename(executable, &staged).context("実行中の実行ファイルを退避できません")?;
        let system_root = env::var_os("SystemRoot").context("SystemRootがありません");
        let launch = system_root.and_then(|root| {
            Command::new(
                std::path::PathBuf::from(root)
                    .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
            )
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .env("EXEC_UNINSTALL_STAGED", &staged)
            .env("EXEC_UNINSTALL_ORIGINAL", executable)
            .env("EXEC_UNINSTALL_RECEIPT", &receipt)
            .env("EXEC_UNINSTALL_READY", &ready)
            .env(
                "EXEC_UNINSTALL_HASH",
                hash.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            )
            .env("EXEC_UNINSTALL_PID", std::process::id().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .spawn()
            .map_err(Into::into)
        });
        let mut child = match launch {
            Ok(child) => child,
            Err(error) => {
                rollback_with_receipt(receipt_directory, || rollback(executable, &staged, hash))
                    .with_context(|| format!("削除ヘルパーを起動できません: {error:#}"))?;
                return Err(error)
                    .context("削除ヘルパーを起動できません。実行ファイルは復元しました");
            }
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if fs::read_to_string(&ready).ok().as_deref() == Some("ready") {
                break;
            }
            let helper_running = matches!(child.try_wait(), Ok(None));
            if !helper_running || Instant::now() >= deadline {
                if let Err(error) = stop(&mut child) {
                    let kept = receipt_directory.keep();
                    bail!(
                        "ヘルパーを停止確認できません: {error}。退避ファイル: {}。記録: {}",
                        staged.display(),
                        kept.join("receipt.json").display()
                    );
                }
                rollback_with_receipt(receipt_directory, || rollback(executable, &staged, hash))
                    .context(
                        "削除ヘルパーの準備を確認できず、実行ファイルの復元にも失敗しました",
                    )?;
                bail!("削除ヘルパーの準備を確認できません。実行ファイルは復元しました");
            }
            thread::sleep(Duration::from_millis(50));
        }
        let kept = receipt_directory.keep();
        eprintln!(
            "削除を予約しました。このプロセス終了後に退避ファイルを削除します（まだ削除完了ではありません）。"
        );
        eprintln!(
            "結果の記録: {}（status: scheduled / deleted / failed）",
            kept.join("receipt.json").display()
        );
        eprintln!("失敗時に手動削除する退避ファイル: {}", staged.display());
        Ok(())
    }
}
