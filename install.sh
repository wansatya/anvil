#!/usr/bin/env bash
#
# Anvil installer — ensures OS dependencies, builds the release binary
# (or fetches a prebuilt one), and installs it locally.
#
# Local source build:
#   ./install.sh [--prefix ~/.local/bin] [--no-deps]
#
# Remote one-liner (once the repo is on GitHub):
#   curl -fsSL https://raw.githubusercontent.com/<owner>/<repo>/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/<owner>/<repo>/main/install.sh | bash -s -- --prefix ~/.local/bin
#
# Dependencies (git, curl, tar, a C linker, Rust) are installed automatically
# per OS unless --no-deps / ANVIL_NO_DEPS=1 is given:
#   Linux   apt/dnf/yum/pacman/apk/zypper (+sudo), rustup for Rust
#   macOS   Xcode Command Line Tools, rustup for Rust
#   Windows Git Bash: winget (Git, Rustup); MSVC/WSL notes on failure
#
# Env overrides:
#   ANVIL_REPO    GitHub "owner/repo" used for remote installs.
#   ANVIL_REF     git ref to clone when doing a remote source build (default: main).
#   ANVIL_NO_DEPS=1  skip automatic dependency installation.
#   PREFIX        install directory (default: $HOME/.local/bin).

set -eu

REPO="${ANVIL_REPO:-}"
REF="${ANVIL_REF:-main}"
PREFIX="${PREFIX:-$HOME/.local/bin}"
NO_DEPS="${ANVIL_NO_DEPS:-0}"
BIN="anvil"
LOG="${TMPDIR:-/tmp}/anvil-install-$$.log"
SPINNER_PID=""

log()  { printf '%s\n' "==> $*" >&2; }
die()  { spin_stop_silent 2>/dev/null || true; printf '%s\n' "error: $*" >&2; exit 1; }

usage() {
  sed -n '2,/^$/p' "$0" | sed 's/^# \?//'
  echo "Usage: $0 [--prefix DIR] [--repo owner/repo] [--ref REF] [--no-deps] [--help]"
}

# --- spinner ------------------------------------------------------------
# Background spinner for long steps; silent when stderr is not a TTY.
# Defined before arg parsing so die() can safely reference it.

spin_start() {
  [ -t 2 ] || { printf '%s...\n' "$1" >&2; return 0; }
  printf '%s ' "$1" >&2
  ( _f='|/-\'; while :; do _i=0; while [ "$_i" -lt 4 ]; do printf '\b%c' "${_f:$_i:1}" >&2; _i=$((_i+1)); sleep 0.15; done; done ) &
  SPINNER_PID=$!
}

spin_stop() { # $1 = ok|fail
  [ -n "${SPINNER_PID:-}" ] || return 0
  kill "$SPINNER_PID" 2>/dev/null || true
  wait "$SPINNER_PID" 2>/dev/null || true
  SPINNER_PID=""
  if [ "$1" = ok ]; then printf '\bdone\n' >&2; else printf '\bFAILED\n' >&2; fi
}

spin_stop_silent() {
  [ -n "${SPINNER_PID:-}" ] || return 0
  kill "$SPINNER_PID" 2>/dev/null || true
  wait "$SPINNER_PID" 2>/dev/null || true
  SPINNER_PID=""
}

trap 'spin_stop_silent' EXIT

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) PREFIX="${2:-}"; shift 2 ;;
    --repo)   REPO="${2:-}"; shift 2 ;;
    --ref)    REF="${2:-}"; shift 2 ;;
    --no-deps) NO_DEPS=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

[ -n "$PREFIX" ] || die "--prefix must not be empty"
mkdir -p "$PREFIX"

# Run "$@" with "$1" shown as spinner message; output goes to $LOG.
# Usage: step "Cloning repo" git clone ...
step() {
  _msg="$1"; shift
  spin_start "$_msg"
  if "$@" >"$LOG" 2>&1; then
    spin_stop ok
  else
    _rc=$?
    spin_stop fail
    tail -n 20 "$LOG" >&2 || true
    die "$_msg failed (exit $_rc, full log: $LOG)"
  fi
}

# --- platform ------------------------------------------------------------

OS=""; ARCH=""
detect_platform() {
  _os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  _arch="$(uname -m)"
  case "$_arch" in
    x86_64|amd64) ARCH="x86_64" ;;
    arm64|aarch64|armv8*) ARCH="aarch64" ;;
    *) die "unsupported architecture: $_arch" ;;
  esac
  case "$_os" in
    linux*) OS="linux" ;;
    darwin*) OS="macos" ;;
    mingw*|msys*|cygwin*) OS="windows" ;;
    *) die "unsupported OS: $_os (Linux / macOS / Windows Git Bash only)" ;;
  esac
}

EXE=""

# --- dependencies ---------------------------------------------------------
# Missing support libs/tools are installed first (forced unless --no-deps).

have() { command -v "$1" >/dev/null 2>&1; }

ensure_deps() {
  if [ "$NO_DEPS" = 1 ]; then
    log "skipping dependency installation (--no-deps)"
    return 0
  fi
  case "$OS" in
    linux) ensure_deps_linux ;;
    macos) ensure_deps_macos ;;
    windows) ensure_deps_windows ;;
  esac
  # Rust everywhere (after system packages so curl exists).
  ensure_rust
  # Final sanity.
  have git || die "git is required but could not be installed automatically"
  have curl || die "curl is required but could not be installed automatically"
  have tar || die "tar is required but could not be installed automatically"
  have cc || have gcc || have clang || die "no C linker (cc/gcc/clang) — install build tools and re-run"
  have cargo || die "cargo is required but could not be installed automatically"
}

sudo_if_needed() {
  if [ "$(id -u 2>/dev/null || echo 0)" -eq 0 ]; then
    printf ''
  elif have sudo; then
    printf 'sudo'
  else
    die "need root to install packages (no sudo found) — install: git curl tar gcc make pkg-config, or re-run as root"
  fi
}

ensure_deps_linux() {
  _missing=""
  have git || _missing="$_missing git"
  have curl || _missing="$_missing curl"
  have tar || _missing="$_missing tar"
  { have cc || have gcc || have clang; } || _missing="$_missing compiler"
  [ -z "$_missing" ] && return 0
  log "installing missing packages:$_missing"
  _sudo="$(sudo_if_needed)"
  pkg() { if [ -n "$_sudo" ]; then $_sudo "$@"; else "$@"; fi; }
  if have apt-get; then
    step "Updating apt" pkg apt-get update
    step "Installing packages" pkg apt-get install -y git curl tar build-essential pkg-config ca-certificates
  elif have dnf; then
    step "Installing packages" pkg dnf install -y git curl tar gcc gcc-c++ make pkgconfig
  elif have yum; then
    step "Installing packages" pkg yum install -y git curl tar gcc gcc-c++ make pkgconfig
  elif have pacman; then
    step "Installing packages" pkg pacman -Sy --noconfirm git curl tar base-devel
  elif have apk; then
    step "Installing packages" pkg apk add git curl tar build-base pkgconfig
  elif have zypper; then
    step "Installing packages" pkg zypper install -y git curl tar gcc make pkg-config
  else
    die "no supported package manager (need:$_missing)"
  fi
}

ensure_deps_macos() {
  # curl + tar ship with macOS; git + C linker come from Command Line Tools.
  if ! have git || ! { have cc || have gcc || have clang; } || ! xcode-select -p >/dev/null 2>&1; then
    log "installing Xcode Command Line Tools (git + compiler)"
    xcode-select --install 2>/dev/null || true
    # The installer is GUI-driven; wait until the tools appear.
    _tries=0
    spin_start "Waiting for Command Line Tools"
    while ! xcode-select -p >/dev/null 2>&1; do
      sleep 10
      _tries=$((_tries+1))
      if [ "$_tries" -ge 120 ]; then
        spin_stop fail
        die "timed out waiting for Command Line Tools — finish the installer dialog and re-run"
      fi
    done
    spin_stop ok
  fi
  have curl || die "curl missing on macOS — reinstall Command Line Tools"
  have tar || die "tar missing on macOS — reinstall Command Line Tools"
}

ensure_deps_windows() {
  # Git Bash / MSYS2 / Cygwin environment.
  if ! have winget && { ! have git || ! have cargo; }; then
    die "winget not found — install Git for Windows and Rust manually, then re-run"
  fi
  if ! have git; then
    step "Installing Git (winget)" winget install -e --silent --accept-package-agreements --accept-source-agreements --id Git.Git
    # Fresh winget installs may not be on PATH in this shell yet.
    for _p in "/c/Program Files/Git/cmd" "/c/Program Files/Git/bin"; do
      [ -x "$_p/git.exe" ] && export PATH="$_p:$PATH"
    done
  fi
  # tar + curl ship with Windows; double-check under this shell.
  have tar || die "tar not found — update Git for Windows / Windows and re-run"
  have curl || die "curl not found — update Windows and re-run"
  if ! { have cc || have gcc || have clang; }; then
    log "WARNING: no C linker found — Rust GNU toolchain needs gcc (Git Bash usually provides it); MSVC needs VS Build Tools"
  fi
}

ensure_rust() {
  if have cargo && have rustc; then
    return 0
  fi
  if [ "$OS" = "windows" ]; then
    step "Installing Rust (winget)" winget install -e --silent --accept-package-agreements --accept-source-agreements --id Rustlang.Rustup
    for _p in "$HOME/.cargo/bin" "/c/Program Files/Rust stable MSVC 1.90/bin"; do
      [ -d "$_p" ] && export PATH="$_p:$PATH"
    done
  else
    step "Installing Rust (rustup)" sh -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable'
    export PATH="$HOME/.cargo/bin:$PATH"
  fi
  have cargo || die "rustup finished but cargo is not on PATH — open a new terminal and re-run"
  step "Checking Rust toolchain" cargo --version
}

# --- fetch / build ----------------------------------------------------------

# Try to install a prebuilt release asset from GitHub. Returns 0 on success.
try_release_asset() {
  _repo="$1"; _dest="$2"
  [ -n "$_repo" ] || return 1
  have curl || return 1
  _plat="${OS}-${ARCH}"
  _url="https://github.com/${_repo}/releases/latest/download/${BIN}-${_plat}.tar.gz"
  _tmp="$(mktemp -d)"
  spin_start "Downloading ${BIN}-${_plat}.tar.gz"
  if curl -fsSL "$_url" -o "$_tmp/anvil.tar.gz" >"$LOG" 2>&1 \
    && tar -xzf "$_tmp/anvil.tar.gz" -C "$_tmp" >>"$LOG" 2>&1 \
    && [ -f "$_tmp/$BIN$EXE" ]; then
    spin_stop ok
    install -m 755 "$_tmp/$BIN$EXE" "$_dest"
    rm -rf "$_tmp"
    return 0
  fi
  spin_stop fail
  rm -rf "$_tmp"
  _url="https://github.com/${_repo}/releases/latest/download/${BIN}-${_plat}${EXE}"
  spin_start "Downloading ${BIN}-${_plat}${EXE}"
  if curl -fsSL "$_url" -o "$_dest" >"$LOG" 2>&1; then
    spin_stop ok
    chmod 755 "$_dest"
    return 0
  fi
  spin_stop fail
  return 1
}

# Fetch sources to a temp dir for remote installs. Echoes the source dir.
fetch_sources() {
  _repo="$1"
  [ -n "$_repo" ] || die "remote install needs --repo owner/repo (or ANVIL_REPO env)"
  _dir="$(mktemp -d)"
  if have git; then
    step "Cloning https://github.com/${_repo}.git@${REF}" git clone --depth 1 --branch "$REF" "https://github.com/${_repo}.git" "$_dir"
  else
    _url="https://github.com/${_repo}/archive/${REF}.tar.gz"
    step "Downloading $_url" curl -fsSL "$_url" -o "$_dir/src.tar.gz"
    step "Unpacking sources" tar -xzf "$_dir/src.tar.gz" -C "$_dir" --strip-components=1
  fi
  printf '%s' "$_dir"
}

build_from_source() {
  _src="$1"
  have cargo || die "cargo not found — re-run without --no-deps so Rust is installed"
  [ -f "$_src/Cargo.toml" ] || die "no Cargo.toml in $_src (not an anvil checkout?)"
  # Prefer locked build; fall back to unlocked (dev checkouts may drift).
  (cd "$_src" && cargo build --release --locked >"$LOG" 2>&1) \
    || (cd "$_src" && cargo build --release >"$LOG" 2>&1) \
    || return 1
}

# --- main ---------------------------------------------------------------

detect_platform
if [ "$OS" = "windows" ]; then EXE=".exe"; else EXE=""; fi

ensure_deps

DEST="$PREFIX/$BIN$EXE"

# Fresh install: remove any previously installed binary first so a stale
# build can never survive a failed install. Scoped to our own files only —
# the rest of $PREFIX is left alone.
for _old in "$DEST" "$PREFIX/$BIN" "$PREFIX/$BIN.exe"; do
  if [ -e "$_old" ] || [ -L "$_old" ]; then
    log "removing previous install: $_old"
    rm -f "$_old"
  fi
done

# Build $_src and install the binary. Fails (non-zero) with log tail on stderr.
build_and_install() {
  _src="$1"
  spin_start "Building release"
  if build_from_source "$_src"; then
    spin_stop ok
  else
    spin_stop fail
    tail -n 20 "$LOG" >&2 || true
    die "build failed (full log: $LOG)"
  fi
  [ -x "$_src/target/release/$BIN$EXE" ] || die "expected binary missing: $_src/target/release/$BIN$EXE"
  install -m 755 "$_src/target/release/$BIN$EXE" "$DEST"
}

if [ -f "./Cargo.toml" ] && [ -d "./crates/anvil-cli" ]; then
  # Local install: we are already inside the repo checkout.
  log "local source install"
  build_and_install "$PWD"
else
  # Remote install (curl | bash): need the GitHub repo coordinates.
  if [ -z "$REPO" ]; then
    # Best effort: derive owner/repo from git remote, if present.
    if have git && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
      _url="$(git remote get-url origin 2>/dev/null || true)"
      REPO="$(printf '%s' "$_url" | sed -nE 's#.*github\.com[:/]([^/]+/[^/]+?)(\.git)?$#\1#p')"
    fi
  fi
  if [ -n "$REPO" ] && try_release_asset "$REPO" "$DEST"; then
    log "installed prebuilt release binary to $DEST"
  else
    [ -n "$REPO" ] || die "no Cargo.toml here and no repo given — set --repo owner/repo (or ANVIL_REPO)"
    log "no prebuilt asset — falling back to source build"
    SRC="$(fetch_sources "$REPO")"
    build_and_install "$SRC"
    rm -rf "$SRC"
  fi
fi

trap - EXIT
log "verifying $DEST"
"$DEST" version || die "installed binary failed to run"
log "installed anvil to $DEST"
case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *) log "NOTE: $PREFIX is not on PATH — add: export PATH=\"$PREFIX:\$PATH\"" ;;
esac
