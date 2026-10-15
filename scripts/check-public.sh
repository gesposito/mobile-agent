#!/usr/bin/env bash
# The publish scan: fails when a tracked file, or a tracked file's name, holds
# what must never be published, so that the repository can stay public. It
# checks:
#
#   - the patterns of scripts/public-patterns.sh: machine paths, iOS device
#     identifiers and their prefix, Android serials, Apple team ids, emails
#     and Mac model identifiers (lock files, being generated, are left out);
#   - the values of the private terms file, in any case, when the file is
#     present: MOBILE_AGENT_PRIVATE_TERMS names it, or else it is the
#     maintainer's copy outside the repository; the maintainer's name may
#     stand in LICENSE's copyright line and the owners' handles in
#     .github/CODEOWNERS, nowhere else;
#   - gitleaks' rules for credentials, over a copy of the tracked files, when
#     gitleaks is installed, with .gitleaks.toml's allowances.
#
# Usage: scripts/check-public.sh
#
# Prints each finding as path:line: what it is, followed by the line for a
# pattern; a private term is named by its line in the list, never by its
# value. Exits 1 when there is a finding, 0 when there is none, 2 when the
# private terms file it was pointed at is missing or this git cannot grep
# with Perl-compatible expressions, and with git's status when git fails.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/public-patterns.sh
. "$here/public-patterns.sh"

root=$(git rev-parse --show-toplevel)
cd "$root"

# git grep answers 1 for no match; a git built without PCRE fails instead.
status=0
git grep -P -q -e 'x' -- ':(exclude)*' > /dev/null 2>&1 || status=$?
if [ "$status" -gt 1 ]; then
  echo "check-public: this git cannot grep with -P (Perl-compatible expressions)" >&2
  exit 2
fi

scratch="$(mktemp -d "${TMPDIR:-/tmp}/check-public.XXXXXX")"
trap 'rm -rf "${scratch:?}"' EXIT
findings="$scratch/findings"
: > "$findings"

# grep_status <status>: 0 or 1 from git grep are its answers; above is a failure.
grep_status() {
  if [ "$1" -gt 1 ]; then
    echo "check-public: git grep failed with status $1" >&2
    exit "$1"
  fi
}

exclude=(':(exclude)*.lock' ':(exclude)scripts/public-patterns.sh')
git ls-files > "$scratch/names"

# The patterns, in the files and in their names.
for i in "${!PUBLIC_REGEXES[@]}"; do
  name=${PUBLIC_NAMES[$i]}
  regex=${PUBLIC_REGEXES[$i]}
  spared=()
  while IFS= read -r path; do
    if [ -n "$path" ]; then spared+=(":(exclude)$path"); fi
  done <<< "${PUBLIC_SPARED[$i]}"
  status=0
  # The spared list may be empty, which bash before 4.4 calls unbound.
  git --no-pager grep -n -I -P -e "$regex" -- . "${exclude[@]}" ${spared[@]+"${spared[@]}"} > "$scratch/hits" || status=$?
  grep_status "$status"
  while IFS= read -r hit; do
    path=${hit%%:*}
    rest=${hit#*:}
    line=${rest%%:*}
    text=${rest#*:}
    printf '%s:%s: %s: %s\n' "$path" "$line" "$name" "$text" >> "$findings"
  done < "$scratch/hits"
  status=0
  (cd "$scratch" && git --no-pager grep --no-index -h -P -e "$regex" -- names) > "$scratch/hits" || status=$?
  grep_status "$status"
  while IFS= read -r path; do
    printf "%s: %s in the file's name\n" "$path" "$name" >> "$findings"
  done < "$scratch/hits"
done

# The private terms.
terms=$(public_terms_file)
# The list's path as the findings show it, the home directory written $HOME.
shown=${terms/#"$HOME"/\$HOME}
if [ -f "$terms" ]; then
  number=0
  while IFS= read -r term || [ -n "$term" ]; do
    number=$((number + 1))
    term=${term%$'\r'}
    case $term in '' | '#'*) continue ;; esac
    status=0
    git --no-pager grep -n -a -i -F -e "$term" -- . ':(exclude)scripts/public-patterns.sh' \
      > "$scratch/hits" || status=$?
    grep_status "$status"
    while IFS= read -r hit; do
      path=${hit%%:*}
      rest=${hit#*:}
      line=${rest%%:*}
      text=${rest#*:}
      if ! public_term_exempt "$path" "$text"; then
        printf '%s:%s: private term on line %s of %s\n' "$path" "$line" "$number" "$shown" >> "$findings"
      fi
    done < "$scratch/hits"
    if grep -q -i -F -e "$term" "$scratch/names"; then
      grep -i -F -e "$term" "$scratch/names" | while IFS= read -r path; do
        printf "%s: private term on line %s of %s in the file's name\n" "$path" "$number" "$shown" >> "$findings"
      done
    fi
  done < "$terms"
elif [ -n "${MOBILE_AGENT_PRIVATE_TERMS:-}" ]; then
  echo "check-public: MOBILE_AGENT_PRIVATE_TERMS names $terms, which does not exist" >&2
  exit 2
else
  echo "check-public: no private terms file at $shown; checking the patterns alone"
fi

# gitleaks, over a copy of the tracked files.
if command -v gitleaks > /dev/null 2>&1; then
  mkdir -p "$scratch/tree"
  # The tracked files present in the checkout, links aside.
  while IFS= read -r -d '' path; do
    if [ -f "$path" ] && [ ! -L "$path" ]; then printf '%s\0' "$path"; fi
  done < <(git ls-files -z) > "$scratch/present"
  tar --null -T "$scratch/present" -cf - | tar -x -m --no-same-permissions -f - -C "$scratch/tree"
  printf '%s\n' '{{range .}}gitleaks: {{.RuleID}} at {{.File}}:{{.StartLine}}' '{{end}}' > "$scratch/report.tmpl"
  # From inside the copy, so that the findings name repo-relative paths.
  status=0
  config=()
  if [ -f "$root/.gitleaks.toml" ]; then config=(--config "$root/.gitleaks.toml"); fi
  (cd "$scratch/tree" && gitleaks dir --no-banner --log-level error --redact ${config[@]+"${config[@]}"} \
    --report-format template --report-template "$scratch/report.tmpl" --report-path "$scratch/gitleaks" .) \
    || status=$?
  if [ "$status" -gt 1 ]; then
    echo "check-public: gitleaks failed with status $status" >&2
    exit "$status"
  fi
  sed -e '/^$/d' "$scratch/gitleaks" >> "$findings"
else
  echo "check-public: gitleaks is not installed; skipping its rules"
fi

if [ -s "$findings" ]; then
  cat "$findings"
  count=$(wc -l < "$findings" | tr -d ' ')
  if [ "$count" = 1 ]; then noun=finding; else noun=findings; fi
  echo "check-public: $count $noun; write a placeholder, \$HOME, \$TMPDIR, <repo> or a repo-relative path instead" >&2
  exit 1
fi
echo "check-public: no tracked file holds a machine path, a device identifier, a team id, an email, a host model or a private term"
