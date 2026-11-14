#!/usr/bin/env bash
# Instaluje MontyBrain dla agentów: serwer MCP `montybrain` oraz skille
# w Claude Code i w Codex. Bezpieczne do ponownego uruchamiania. Po udanej
# instalacji usuwa pozostałości po Morwisku (rejestrację `morwisko`, binarkę
# morwisko-mcp i skille morwisko-*), ale tylko te wgrane przez ten skrypt.
#
#   ./install.sh                 wszystko: build, rejestracja MCP, skille
#   ./install.sh --no-build      bez budowania (użyj już zainstalowanej binarki)
#   ./install.sh --only claude   tylko Claude Code (albo: --only codex)
#   ./install.sh --no-skills     tylko serwer MCP
#   ./install.sh --vault DIR     inny folder danych niż ~/MontyBrain
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
NAME="montybrain"
BIN_DIR="${MONTYBRAIN_BIN_DIR:-${MORWISKO_BIN_DIR:-$HOME/.local/bin}}"
BIN="$BIN_DIR/montybrain-mcp"
MARKER=".montybrain-installed"
# Przed zmianą nazwy (Morwisko): sprzątane po udanej instalacji, tylko nasze.
OLD_NAME="morwisko"
OLD_BIN="$BIN_DIR/morwisko-mcp"
OLD_MARKER=".morwisko-installed"
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
    -h|--help) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
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
# montybrain-zadania -> morwisko-zadania
old_skill() { printf '%s\n' "$OLD_NAME-${1#"$NAME"-}"; }

# Polecenie serwera $2 zarejestrowanego w $1 (puste, gdy go nie ma).
registered_command() {
  "$1" mcp get "$2" 2>/dev/null | awk 'tolower($1) == "command:" { sub(/^[^:]*:[ \t]*/, ""); print; exit }' || true
}

# Stara rejestracja `morwisko` w $1, jeśli wskazuje na naszą binarkę morwisko-mcp
# (wypisuje jej polecenie; nic, gdy jej nie ma albo jest cudza).
old_registration() {
  have "$1" || return 0
  local cmd; cmd="$(registered_command "$1" "$OLD_NAME")"
  case "$cmd" in */morwisko-mcp|morwisko-mcp) printf '%s\n' "$cmd" ;; esac
}

# --- status -----------------------------------------------------------------
status() {
  if [ -x "$BIN" ]; then say "binarka:      $BIN"; else say "binarka:      brak ($BIN)"; fi
  if [ -e "$OLD_BIN" ]; then say "binarka:      $OLD_BIN (stara, Morwisko)"; fi
  if have claude; then
    if claude mcp get "$NAME" >/dev/null 2>&1; then say "Claude Code:  serwer MCP zarejestrowany"; else say "Claude Code:  serwer MCP niezarejestrowany"; fi
    if claude mcp get "$OLD_NAME" >/dev/null 2>&1; then say "Claude Code:  stary serwer MCP $OLD_NAME zarejestrowany ($(registered_command claude "$OLD_NAME"))"; fi
  else say "Claude Code:  brak polecenia claude"; fi
  if have codex; then
    if codex mcp get "$NAME" >/dev/null 2>&1; then say "Codex:        serwer MCP zarejestrowany"; else say "Codex:        serwer MCP niezarejestrowany"; fi
    if codex mcp get "$OLD_NAME" >/dev/null 2>&1; then say "Codex:        stary serwer MCP $OLD_NAME zarejestrowany ($(registered_command codex "$OLD_NAME"))"; fi
  else say "Codex:        brak polecenia codex"; fi
  for n in $(skill_names); do
    c="-"; x="-"
    [ -f "$CLAUDE_SKILLS/$n/SKILL.md" ] && c="Claude Code"
    [ -f "$CODEX_SKILLS/$n/SKILL.md" ] && x="Codex"
    say "skill $n: $c, $x"
  done
  for n in $(skill_names); do
    o="$(old_skill "$n")"; c="-"; x="-"
    [ -f "$CLAUDE_SKILLS/$o/SKILL.md" ] && c="Claude Code"
    [ -f "$CODEX_SKILLS/$o/SKILL.md" ] && x="Codex"
    if [ "$c$x" != "--" ]; then say "skill $o (stary): $c, $x"; fi
  done
}

# --- pozostałości po Morwisku -------------------------------------------------
# Usuwa starą rejestrację, skille i binarkę, ale tylko nasze: rejestrację, gdy
# wskazuje na morwisko-mcp, skille ze znacznikiem $OLD_MARKER, binarkę, gdy
# żadna rejestracja już jej nie używa. Cudze rzeczy zostają (z komunikatem).
# $1 = keep-binary: bez usuwania binarki (--uninstall).
remove_old() {
  local cli label n o root
  for cli in claude codex; do
    want "$cli" && have "$cli" || continue
    case "$cli" in claude) label="Claude Code" ;; codex) label="Codex" ;; esac
    if [ -n "$(old_registration "$cli")" ]; then
      if { [ "$cli" = claude ] && claude mcp remove "$OLD_NAME" -s user >/dev/null 2>&1; } \
        || { [ "$cli" = codex ] && codex mcp remove "$OLD_NAME" >/dev/null 2>&1; }; then
        say "$label: usunięto starą rejestrację serwera MCP $OLD_NAME"
      else
        echo "$label: nie udało się usunąć starej rejestracji $OLD_NAME (usuń ręcznie: $cli mcp remove $OLD_NAME)." >&2
        failed=1
      fi
    elif "$cli" mcp get "$OLD_NAME" >/dev/null 2>&1; then
      say "$label: zostawiam serwer MCP $OLD_NAME — nie wskazuje na morwisko-mcp"
    fi
  done
  if [ "$skills" = 1 ]; then
    for n in $(skill_names); do
      o="$(old_skill "$n")"
      for root in "$CLAUDE_SKILLS" "$CODEX_SKILLS"; do
        case "$root" in "$CLAUDE_SKILLS") want claude || continue ;; "$CODEX_SKILLS") want codex || continue ;; esac
        [ -e "$root/$o" ] || continue
        if [ -f "$root/$o/$OLD_MARKER" ]; then
          rm -rf "${root:?}/$o" && say "usunięto stary skill: $root/$o"
        else
          say "zostawiam $root/$o — nie wgrał go ten skrypt"
        fi
      done
    done
  fi
  # Binarka: dopiero gdy ani Claude Code, ani Codex nie mają już rejestracji, która jej używa.
  if [ "${1:-}" != keep-binary ] && [ -e "$OLD_BIN" ]; then
    if [ -n "$(old_registration claude)$(old_registration codex)" ]; then
      say "zostawiam $OLD_BIN — używa jej jeszcze rejestracja $OLD_NAME"
    elif rm -f "$OLD_BIN"; then
      say "usunięto starą binarkę: $OLD_BIN"
    else
      echo "Nie udało się usunąć $OLD_BIN." >&2
      failed=1
    fi
  fi
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
        if [ -f "$root/$n/$MARKER" ]; then rm -rf "${root:?}/$n"; say "usunięto skill: $root/$n"; fi
      done
    done
  fi
  # Stare (Morwisko) rejestracje i skille, jeśli są nasze; binarki zostają jak dotąd.
  remove_old keep-binary
  say "Folder danych i aplikacja zostają bez zmian."
}

# --- install ----------------------------------------------------------------
install_binary() {
  if [ "$build" = 1 ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
    have cargo || { echo "Brak cargo. Zainstaluj Rust (https://rustup.rs) albo użyj --no-build." >&2; exit 1; }
    say "Buduję montybrain-mcp…"
    cargo build --release --locked -p montybrain-mcp --manifest-path "$REPO/Cargo.toml"
    mkdir -p "$BIN_DIR"
    # Kopia przez plik tymczasowy: działający serwer nie dostaje podmienionej binarki w połowie.
    cp "$REPO/target/release/montybrain-mcp" "$BIN.new"
    mv -f "$BIN.new" "$BIN"
  fi
  [ -x "$BIN" ] || { echo "Nie ma binarki $BIN. Uruchom bez --no-build." >&2; exit 1; }
  say "binarka: $BIN"
}

claude_add() {  # $1 = binarka, $2 = folder danych albo pusty
  if [ -n "$2" ]; then
    claude mcp add --scope user -e "MONTYBRAIN_DIR=$2" "$NAME" -- "$1" >/dev/null
  else
    claude mcp add --scope user "$NAME" -- "$1" >/dev/null
  fi
}

codex_add() {
  if [ -n "$2" ]; then
    codex mcp add --env "MONTYBRAIN_DIR=$2" "$NAME" -- "$1" >/dev/null
  else
    codex mcp add "$NAME" -- "$1" >/dev/null
  fi
}

register_claude() {
  have claude || { say "Claude Code: brak polecenia claude, pomijam"; return 0; }
  # Poprzednia rejestracja — do przywrócenia, jeśli nowa się nie powiedzie.
  local old; old="$(registered_command claude "$NAME")"
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
  local old; old="$(registered_command codex "$NAME")"
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
    if [ -e "$dest" ] && [ ! -f "$dest/$MARKER" ]; then
      say "$label: pomijam skill $n — w $dest jest już coś, czego nie wgrał ten skrypt"
      continue
    fi
    # Folder skilla jest zastępowany w całości (własne pliki dodane do niego znikną),
    # ale dopiero gdy nowa kopia jest kompletna: nieudane kopiowanie zostawia starą wersję.
    local stage="$dest.new.$$" old="$dest.old.$$"
    rm -rf "$stage" "$old"
    if ! { mkdir -p "$stage" && cp -R "$REPO/skills/$n/." "$stage/" && [ -f "$stage/SKILL.md" ] && : > "$stage/$MARKER"; }; then
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

failed=0
case "$action" in
  status) status; exit 0 ;;
  uninstall) uninstall; exit "$failed" ;;
esac

if [ "$skills" = 1 ] && [ -z "$(skill_names)" ]; then
  echo "Nie znaleziono skilli w $REPO/skills. Uruchom skrypt z repozytorium MontyBrain albo dodaj --no-skills." >&2
  exit 1
fi

install_binary
if want claude; then register_claude || failed=1; fi
if want codex; then register_codex || failed=1; fi
if [ "$skills" = 1 ]; then
  if want claude; then install_skills "$CLAUDE_SKILLS" "Claude Code"; fi
  if want codex; then install_skills "$CODEX_SKILLS" "Codex"; fi
fi
# Pozostałości po Morwisku dopiero po udanej instalacji: przy błędzie zostaje
# stara, działająca konfiguracja.
if [ "$failed" = 0 ]; then remove_old; fi
say ""
if [ "$failed" = 1 ]; then
  echo "Instalacja zakończona z błędami (patrz wyżej)." >&2
  exit 1
fi
say "Gotowe. Narzędzia i skille zobaczą sesje uruchomione od teraz."
# Ta sama kolejność co w aplikacji: zmienne, potem ~/MontyBrain, potem istniejące ~/Morwisko.
data_dir="${vault:-${MONTYBRAIN_DIR:-${MORWISKO_DIR:-}}}"
if [ -z "$data_dir" ]; then
  if [ -d "$HOME/MontyBrain" ] || [ ! -d "$HOME/Morwisko" ]; then data_dir="$HOME/MontyBrain"; else data_dir="$HOME/Morwisko"; fi
fi
say "Folder danych: $data_dir"
