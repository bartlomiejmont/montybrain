#!/usr/bin/env bash
# Instaluje Morwisko dla agentów: serwer MCP `morwisko` oraz skille
# w Claude Code i w Codex. Bezpieczne do ponownego uruchamiania.
#
#   ./install.sh                 wszystko: build, rejestracja MCP, skille
#   ./install.sh --no-build      bez budowania (użyj już zainstalowanej binarki)
#   ./install.sh --only claude   tylko Claude Code (albo: --only codex)
#   ./install.sh --no-skills     tylko serwer MCP
#   ./install.sh --vault DIR     inny folder danych niż ~/Morwisko
#   ./install.sh --uninstall     usuwa rejestrację MCP i skille (danych nie rusza)
#   ./install.sh --status        pokazuje, co jest zainstalowane
set -euo pipefail

# Folder repozytorium, także gdy skrypt uruchomiono przez dowiązanie symboliczne.
SELF="${BASH_SOURCE[0]}"
while [ -L "$SELF" ]; do
  LINK="$(readlink "$SELF")"
  case "$LINK" in /*) SELF="$LINK" ;; *) SELF="$(dirname "$SELF")/$LINK" ;; esac
done
REPO="$(cd "$(dirname "$SELF")" && pwd -P)"
NAME="morwisko"
BIN_DIR="${MORWISKO_BIN_DIR:-$HOME/.local/bin}"
BIN="$BIN_DIR/morwisko-mcp"
CLAUDE_SKILLS="${CLAUDE_SKILLS_DIR:-$HOME/.claude/skills}"
CODEX_SKILLS="${CODEX_SKILLS_DIR:-$HOME/.codex/skills}"

build=1; skills=1; only=""; vault=""; action="install"
while [ $# -gt 0 ]; do
  case "$1" in
    --no-build) build=0 ;;
    --no-skills) skills=0 ;;
    --only|--vault)
      [ $# -ge 2 ] || { echo "Opcja $1 wymaga wartości (zobacz --help)" >&2; exit 2; }
      if [ "$1" = "--only" ]; then only="$2"; else vault="$2"; fi
      shift ;;
    --uninstall) action="uninstall" ;;
    --status) action="status" ;;
    -h|--help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Nieznana opcja: $1 (zobacz --help)" >&2; exit 2 ;;
  esac
  shift
done
case "$only" in ""|claude|codex) ;; *) echo "--only przyjmuje: claude albo codex" >&2; exit 2 ;; esac
if [ -n "$vault" ]; then
  case "$vault" in /*) ;; *) vault="$PWD/$vault" ;; esac
fi

want() { [ -z "$only" ] || [ "$only" = "$1" ]; }
have() { command -v "$1" >/dev/null 2>&1; }
say()  { printf '%s\n' "$*"; }
skill_names() { for d in "$REPO"/skills/*/; do [ -f "$d/SKILL.md" ] && basename "$d"; done; }

# --- status -----------------------------------------------------------------
status() {
  if [ -x "$BIN" ]; then say "binarka:      $BIN"; else say "binarka:      brak ($BIN)"; fi
  if have claude; then
    if claude mcp get "$NAME" >/dev/null 2>&1; then say "Claude Code:  serwer MCP zarejestrowany"; else say "Claude Code:  serwer MCP niezarejestrowany"; fi
  else say "Claude Code:  brak polecenia claude"; fi
  if have codex; then
    if codex mcp get "$NAME" >/dev/null 2>&1; then say "Codex:        serwer MCP zarejestrowany"; else say "Codex:        serwer MCP niezarejestrowany"; fi
  else say "Codex:        brak polecenia codex"; fi
  for n in $(skill_names); do
    c="-"; x="-"
    [ -f "$CLAUDE_SKILLS/$n/SKILL.md" ] && c="Claude Code"
    [ -f "$CODEX_SKILLS/$n/SKILL.md" ] && x="Codex"
    say "skill $n: $c, $x"
  done
}

# --- uninstall --------------------------------------------------------------
uninstall() {
  if want claude && have claude; then
    claude mcp remove "$NAME" -s user >/dev/null 2>&1 && say "Claude Code: usunięto serwer MCP" || say "Claude Code: serwera MCP nie było"
  fi
  if want codex && have codex; then
    codex mcp remove "$NAME" >/dev/null 2>&1 && say "Codex: usunięto serwer MCP" || say "Codex: serwera MCP nie było"
  fi
  if [ "$skills" = 1 ]; then
    for n in $(skill_names); do
      # Usuwamy tylko skille, które sami wgraliśmy (znacznik w folderze).
      for root in "$CLAUDE_SKILLS" "$CODEX_SKILLS"; do
        case "$root" in "$CLAUDE_SKILLS") want claude || continue ;; "$CODEX_SKILLS") want codex || continue ;; esac
        if [ -f "$root/$n/.morwisko-installed" ]; then rm -rf "${root:?}/$n"; say "usunięto skill: $root/$n"; fi
      done
    done
  fi
  say "Folder danych i aplikacja zostają bez zmian."
}

# --- install ----------------------------------------------------------------
install_binary() {
  if [ "$build" = 1 ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
    have cargo || { echo "Brak cargo. Zainstaluj Rust (https://rustup.rs) albo użyj --no-build." >&2; exit 1; }
    say "Buduję morwisko-mcp…"
    cargo build --release --locked -p morwisko-mcp --manifest-path "$REPO/Cargo.toml"
    mkdir -p "$BIN_DIR"
    # Kopia przez plik tymczasowy: działający serwer nie dostaje podmienionej binarki w połowie.
    cp "$REPO/target/release/morwisko-mcp" "$BIN.new"
    mv -f "$BIN.new" "$BIN"
  fi
  [ -x "$BIN" ] || { echo "Nie ma binarki $BIN. Uruchom bez --no-build." >&2; exit 1; }
  say "binarka: $BIN"
}

# Polecenie obecnie zarejestrowanego serwera (puste, gdy go nie ma) — do przywrócenia,
# jeśli nowa rejestracja się nie powiedzie.
registered_command() {
  "$1" mcp get "$NAME" 2>/dev/null | awk 'tolower($1) == "command:" { sub(/^[^:]*:[ \t]*/, ""); print; exit }' || true
}

claude_add() {  # $1 = binarka, $2 = folder danych albo pusty
  if [ -n "$2" ]; then
    claude mcp add --scope user -e "MORWISKO_DIR=$2" "$NAME" -- "$1" >/dev/null
  else
    claude mcp add --scope user "$NAME" -- "$1" >/dev/null
  fi
}

codex_add() {
  if [ -n "$2" ]; then
    codex mcp add --env "MORWISKO_DIR=$2" "$NAME" -- "$1" >/dev/null
  else
    codex mcp add "$NAME" -- "$1" >/dev/null
  fi
}

register_claude() {
  have claude || { say "Claude Code: brak polecenia claude, pomijam"; return 0; }
  local old; old="$(registered_command claude)"
  claude mcp remove "$NAME" -s user >/dev/null 2>&1 || true
  if claude_add "$BIN" "$vault"; then
    say "Claude Code: serwer MCP zarejestrowany (zakres: użytkownik)"
    return 0
  fi
  echo "Claude Code: rejestracja serwera MCP nie powiodła się." >&2
  if [ -n "$old" ] && claude_add "$old" ""; then
    echo "Claude Code: przywrócono poprzednią rejestrację ($old)." >&2
  else
    echo "Claude Code: serwer MCP nie jest teraz zarejestrowany. Ręcznie: claude mcp add --scope user $NAME -- $BIN" >&2
  fi
  return 1
}

register_codex() {
  have codex || { say "Codex: brak polecenia codex, pomijam"; return 0; }
  local old; old="$(registered_command codex)"
  codex mcp remove "$NAME" >/dev/null 2>&1 || true
  if codex_add "$BIN" "$vault"; then
    say "Codex: serwer MCP zarejestrowany"
    return 0
  fi
  echo "Codex: rejestracja serwera MCP nie powiodła się." >&2
  if [ -n "$old" ] && codex_add "$old" ""; then
    echo "Codex: przywrócono poprzednią rejestrację ($old)." >&2
  else
    echo "Codex: serwer MCP nie jest teraz zarejestrowany. Ręcznie: codex mcp add $NAME -- $BIN" >&2
  fi
  return 1
}

install_skills() {
  local root="$1" label="$2" n dest
  mkdir -p "$root"
  for n in $(skill_names); do
    dest="$root/$n"
    if [ -e "$dest" ] && [ ! -f "$dest/.morwisko-installed" ]; then
      say "$label: pomijam skill $n — w $dest jest już coś, czego nie wgrał ten skrypt"
      continue
    fi
    # Folder skilla jest zastępowany w całości (własne pliki dodane do niego znikną),
    # ale dopiero gdy nowa kopia jest kompletna: nieudane kopiowanie zostawia starą wersję.
    local stage="$dest.new.$$" old="$dest.old.$$"
    rm -rf "$stage" "$old"
    if ! { mkdir -p "$stage" && cp -R "$REPO/skills/$n/." "$stage/" && [ -f "$stage/SKILL.md" ] && : > "$stage/.morwisko-installed"; }; then
      rm -rf "$stage"
      echo "$label: nie udało się skopiować skilla $n; poprzednia wersja została." >&2
      failed=1
      continue
    fi
    if [ -e "$dest" ]; then mv "$dest" "$old"; fi
    if mv "$stage" "$dest"; then
      rm -rf "$old"
      say "$label: skill $n"
    else
      [ -e "$old" ] && mv "$old" "$dest"
      rm -rf "$stage"
      echo "$label: nie udało się podmienić skilla $n; poprzednia wersja została." >&2
      failed=1
    fi
  done
}

case "$action" in
  status) status; exit 0 ;;
  uninstall) uninstall; exit 0 ;;
esac

failed=0
if [ "$skills" = 1 ] && [ -z "$(skill_names)" ]; then
  echo "Nie znaleziono skilli w $REPO/skills. Uruchom skrypt z repozytorium Morwiska albo dodaj --no-skills." >&2
  exit 1
fi

install_binary
if want claude; then register_claude || failed=1; fi
if want codex; then register_codex || failed=1; fi
if [ "$skills" = 1 ]; then
  if want claude; then install_skills "$CLAUDE_SKILLS" "Claude Code"; fi
  if want codex; then install_skills "$CODEX_SKILLS" "Codex"; fi
fi
say ""
if [ "$failed" = 1 ]; then
  echo "Instalacja zakończona z błędami (patrz wyżej)." >&2
  exit 1
fi
say "Gotowe. Narzędzia i skille zobaczą sesje uruchomione od teraz."
say "Folder danych: ${vault:-${MORWISKO_DIR:-$HOME/Morwisko}}"
