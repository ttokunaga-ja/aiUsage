# irm https://raw.githubusercontent.com/ttokunaga-ja/aiUsage/main/install.ps1 | iex
# BIN_DIR overrides the destination. AI_USAGE_INSTALL_NO_PATH=1 is for isolated tests.
# Keep this scoped and never call exit: iex runs inside the user's PowerShell.
& {
    $ErrorActionPreference = 'Stop'
    $base = 'https://github.com/ttokunaga-ja/aiUsage/releases/latest/download'
    $bin = if ($env:BIN_DIR) { $env:BIN_DIR } else { Join-Path $env:USERPROFILE '.local\bin' }
    if (-not [IO.Path]::IsPathRooted($bin)) { throw 'BIN_DIR は絶対パスにしてください' }
    $bin = [IO.Path]::GetFullPath($bin)
    if ($bin -ne [IO.Path]::GetPathRoot($bin)) { $bin = $bin.TrimEnd('\') }
    if ($bin.Contains(';') -or $bin.Contains("`r") -or $bin.Contains("`n")) { throw 'BIN_DIR に PATH の区切り文字や改行は使えません' }
    if ($env:AI_USAGE_INSTALL_NO_PATH -ne '1' -and $bin.Contains('%')) { throw 'PATH を自動設定する場合、BIN_DIR に % は使えません' }
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $temp = Join-Path ([IO.Path]::GetTempPath()) ('aiUsage-install-' + [Guid]::NewGuid().ToString('N'))
    $stage = $null
    try {
        New-Item -ItemType Directory -Path $temp | Out-Null
        $asset = 'aiUsage-windows-x64.exe'
        $download = Join-Path $temp $asset
        $manifest = Join-Path $temp 'SHA256SUMS'
        Invoke-WebRequest -UseBasicParsing -TimeoutSec 300 -Uri "$base/$asset" -OutFile $download
        Invoke-WebRequest -UseBasicParsing -TimeoutSec 300 -Uri "$base/SHA256SUMS" -OutFile $manifest
        $entries = @(Get-Content -LiteralPath $manifest | Where-Object { $_ -match '\s+\*?aiUsage-windows-x64\.exe\s*$' })
        if ($entries.Count -ne 1 -or $entries[0] -notmatch '^([0-9a-fA-F]{64}) [ *]aiUsage-windows-x64\.exe$') {
            throw 'SHA256SUMS の Windows 用項目が不正または重複しています'
        }
        $expected = $Matches[1]
        if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ine $expected) { throw 'SHA-256 が一致しません' }
        New-Item -ItemType Directory -Path $bin -Force | Out-Null
        $exe = Join-Path $bin 'aiUsage.exe'
        if (Test-Path -LiteralPath $exe) {
            $existing = Get-Item -LiteralPath $exe -Force
            if ($existing.PSIsContainer -or ($existing.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw 'インストール先が通常ファイルではないため変更しません'
            }
        }
        $stage = Join-Path $bin ('.aiUsage-install-' + [Guid]::NewGuid().ToString('N') + '.exe')
        [IO.File]::Copy($download, $stage, $false)
        $version = @(& $stage --version)
        if ($LASTEXITCODE -ne 0 -or $version.Count -ne 1 -or $version[0] -notmatch '^aiUsage [0-9]+\.[0-9]+\.[0-9]+$') {
            throw '実行ファイルのバージョンを確認できません。既存のインストールは変更していません'
        }
        $backup = $null
        if (Test-Path -LiteralPath $exe) {
            # PowerShell 5.1 converts a null string argument to an empty path.
            $backup = Join-Path $bin ('.aiUsage-backup-' + [Guid]::NewGuid().ToString('N') + '.exe')
            try { [IO.File]::Replace($stage, $exe, $backup) }
            catch {
                if (Test-Path -LiteralPath $backup) { Write-Warning "既存の実行ファイルの退避先を保持しました: $backup" }
                throw
            }
        } else { [IO.File]::Move($stage, $exe) }
        $stage = $null
        if ($backup -and (Test-Path -LiteralPath $backup)) {
            try { Remove-Item -LiteralPath $backup -Force }
            catch { Write-Warning "インストール済みですが、旧版の退避先を削除できませんでした: $backup" }
        }
        Write-Host "インストールしました: $exe ($($version[0]))"

        if ($env:AI_USAGE_INSTALL_NO_PATH -ne '1') {
            $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
            try {
                $raw = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                $present = @($raw -split ';' | Where-Object { [Environment]::ExpandEnvironmentVariables($_).Trim().Trim('"').TrimEnd('\') -ieq $bin })
                if ($present.Count -eq 0) {
                    $kind = if ($key.GetValueNames() -contains 'Path') { $key.GetValueKind('Path') } else { [Microsoft.Win32.RegistryValueKind]::ExpandString }
                    $separator = if ($raw.Length -eq 0 -or $raw.EndsWith(';')) { '' } else { ';' }
                    $key.SetValue('Path', ($raw + $separator + $bin), $kind)
                    # Notify Explorer without writing any unrelated environment variable.
                    if (-not ('AiUsageInstaller.EnvironmentNotification' -as [type])) {
                        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace AiUsageInstaller {
    public static class EnvironmentNotification {
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, UIntPtr wParam, string lParam, uint flags, uint timeout, out UIntPtr result);
    }
}
'@
                    }
                    $result = [UIntPtr]::Zero
                    [void][AiUsageInstaller.EnvironmentNotification]::SendMessageTimeout([IntPtr]0xffff, 0x1a, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$result)
                    Write-Host "ユーザー PATH に $bin を追加しました"
                }
            } finally { $key.Close() }
            $current = @($env:Path -split ';' | Where-Object { [Environment]::ExpandEnvironmentVariables($_).Trim().Trim('"').TrimEnd('\') -ieq $bin })
            if ($current.Count -eq 0) { $env:Path = $bin + ';' + $env:Path }
            Write-Host 'この PowerShell で aiUsage を使えます。'
        }
    } finally {
        if ($stage -and (Test-Path -LiteralPath $stage)) { Remove-Item -LiteralPath $stage -Force }
        if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
    }
}
