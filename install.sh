#!/bin/sh
# curl -fsSL https://raw.githubusercontent.com/ttokunaga-ja/aiUsage/main/install.sh | sh
# BIN_DIR overrides the destination. AI_USAGE_INSTALL_NO_PATH=1 is for isolated tests.
set -eu

if [ "$(uname -s)" != Darwin ]; then
  echo 'install.sh: macOS 用です。Windows は install.ps1 を使ってください' >&2
  exit 1
fi
bin=${BIN_DIR:-"$HOME/.local/bin"}
case "$bin" in /*) ;; *) echo 'BIN_DIR は絶対パスにしてください' >&2; exit 1 ;; esac
case "$bin" in *:*) echo 'BIN_DIR に PATH の区切り文字 : は使えません' >&2; exit 1 ;; esac
# A newline cannot be represented safely in the profile entry.
case "$bin" in *'
'*) echo 'BIN_DIR に改行は使えません' >&2; exit 1 ;; esac
base=https://github.com/ttokunaga-ja/aiUsage/releases/latest/download
tmp=$(mktemp -d)
stage=
trap 'rm -rf "$tmp"; if [ -n "$stage" ]; then rm -f "$stage"; fi' EXIT
trap 'exit 1' HUP INT TERM
curl --proto '=https' --tlsv1.2 -fsSL -o "$tmp/aiUsage-macos" "$base/aiUsage-macos"
curl --proto '=https' --tlsv1.2 -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"
# Exactly one entry is required, including when a duplicate has the same digest.
awk '$2 == "aiUsage-macos" || $2 == "*aiUsage-macos" { print }' "$tmp/SHA256SUMS" > "$tmp/match"
if [ "$(wc -l < "$tmp/match" | tr -d ' ')" != 1 ] ||
   ! LC_ALL=C grep -Eq '^[0-9a-fA-F]{64} [ *]aiUsage-macos$' "$tmp/match" ||
   ! (cd "$tmp" && shasum -a 256 -c match >/dev/null); then
  echo 'install.sh: SHA256SUMS の項目が不正、または SHA-256 が一致しません' >&2
  exit 1
fi
chmod 755 "$tmp/aiUsage-macos"
version=$("$tmp/aiUsage-macos" --version)
if [ "$(printf '%s\n' "$version" | wc -l | tr -d ' ')" != 1 ] ||
   ! printf '%s\n' "$version" | LC_ALL=C grep -Eq '^aiUsage [0-9]+\.[0-9]+\.[0-9]+$'; then
  echo 'install.sh: 実行ファイルのバージョンを確認できません' >&2
  exit 1
fi
mkdir -p "$bin"
if [ -L "$bin/aiUsage" ] || { [ -e "$bin/aiUsage" ] && [ ! -f "$bin/aiUsage" ]; }; then
  echo 'install.sh: インストール先が通常ファイルではないため変更しません' >&2
  exit 1
fi
stage=$(mktemp "$bin/.aiUsage-install.XXXXXXXX")
cp "$tmp/aiUsage-macos" "$stage"
chmod 755 "$stage"
mv -f "$stage" "$bin/aiUsage"
stage=
echo "インストールしました: $bin/aiUsage ($version)"

if [ "${AI_USAGE_INSTALL_NO_PATH:-0}" != 1 ]; then
  case "${SHELL:-}" in
    */zsh) profile=${ZDOTDIR:-"$HOME"}/.zshrc ;;
    */bash)
      if [ -f "$HOME/.bash_profile" ]; then profile=$HOME/.bash_profile
      elif [ -f "$HOME/.bash_login" ]; then profile=$HOME/.bash_login
      elif [ -f "$HOME/.profile" ]; then profile=$HOME/.profile
      else profile=$HOME/.bash_profile; fi ;;
    *) profile=$HOME/.profile ;;
  esac
  # Literal single quoting also supports spaces, dollar signs and apostrophes.
  quoted=$(printf '%s' "$bin" | sed "s/'/'\\\\''/g")
  entry="case :\$PATH: in *:'$quoted':*) ;; *) export PATH='$quoted':\$PATH ;; esac # aiUsage installer"
  if [ ! -f "$profile" ] || ! grep -Fqx "$entry" "$profile"; then
    mkdir -p "$(dirname "$profile")"
    printf '\n%s\n' "$entry" >> "$profile"
    echo "PATH を設定しました: $profile"
  fi
  echo '新しいターミナルを開くと aiUsage を使えます。'
fi
