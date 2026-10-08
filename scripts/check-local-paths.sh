#!/usr/bin/env bash
# Fails when a tracked file names a path particular to the machine it was
# written on, which means nothing in another checkout and can give away an
# account name. It looks for:
#   - a macOS home directory: /Users/ and an account name (a placeholder such
#     as /Users/<user> passes);
#   - Claude Code's per-user directory: ~/.claude or $HOME/.claude;
#   - a Claude Code scratch directory: /tmp/claude-<uid>, also under /private;
#   - the macOS per-user temporary directory under /var/folders;
#   - the directory that holds this repository's worktrees, mobile-agent-wt;
#   - an iOS device identifier (0000XXXX-XXXXXXXXXXXXXXXX), which names one
#     person's phone; write the model and keep the identifier in the
#     environment and the reports' provenance;
#   - an email address, other than one under example.com, .org or .net and
#     Kotlin's this@Class.
# Write $HOME, $TMPDIR, <repo> for the checkout, or a repo-relative path
# instead. Lock files are generated, so they are skipped; so is this script,
# which spells out the patterns.
#
# Usage: scripts/check-local-paths.sh
#
# Prints each offending line as path:line:text and exits 1 when there is one,
# 0 when there is none, and with git's status when git fails.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

home='/Users/[A-Za-z0-9_][A-Za-z0-9._-]*'
claude='(~|\$HOME)/\.claude'
scratch='/tmp/claude-[0-9]+'
tmpdir='/var/folders/'
worktrees='mobile-agent-wt'
udid='0000[0-9A-Fa-f]{4}-[0-9A-Fa-f]{16}'
email='[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}'

# git grep exits 0 on a match, 1 on none and above 1 on an error.
status=0
found=$(git --no-pager grep -nE "$home|$claude|$scratch|$tmpdir|$worktrees|$udid" -- . \
  ':(exclude)*.lock' ':(exclude)scripts/check-local-paths.sh') || status=$?
# Email addresses, less the reserved example domains and Kotlin's this@Class.
mail_status=0
mail=$(git --no-pager grep -nE "$email" -- . ':(exclude)*.lock' ':(exclude)scripts/check-local-paths.sh' \
  | grep -vE '@example\.(com|org|net)\b|this@' ) || mail_status=$?
if [ "$mail_status" = 0 ]; then
  found=$(printf '%s\n%s' "$found" "$mail" | sed '/^$/d')
  status=0
fi

case $status in
  0)
    printf '%s\n' "$found"
    count=$(printf '%s\n' "$found" | wc -l | tr -d ' ')
    if [ "$count" = 1 ]; then lines=line; else lines=lines; fi
    echo "check-local-paths: machine-local paths on $count tracked $lines; write \$HOME, \$TMPDIR, <repo> or a repo-relative path instead" >&2
    exit 1
    ;;
  1)
    echo "check-local-paths: no tracked file names a machine-local path"
    ;;
  *)
    echo "check-local-paths: git grep failed with status $status" >&2
    exit "$status"
    ;;
esac
