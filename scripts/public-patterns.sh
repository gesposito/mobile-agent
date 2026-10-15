# The publish scan's patterns, sourced by scripts/check-public.sh, which scans
# the tracked files, and scripts/check-history.sh, which scans every commit.
# Each is a name and a Perl-compatible regular expression, as `git grep -P`
# reads it, for something that must never be published:
#
#   - a home directory: /Users/ or /home/ and an account name; placeholders
#     such as /Users/<user>, /Users/Shared, /home/me, /home/user and
#     /home/runner pass;
#   - Claude Code's per-user directory, ~/.claude or $HOME/.claude, and its
#     scratch directories, /tmp/claude-<uid>, also under /private;
#   - the macOS per-user temporary directory under /var/folders;
#   - the directory that holds this repository's worktrees;
#   - an iOS device identifier (0000XXXX-XXXXXXXXXXXXXXXX), which names one
#     person's phone, and its eight-digit prefix alone, whose chip id starts
#     with a digit from 1 to 9, as no hex dump's zero-padded word does;
#   - an Android device's serial where the tree writes one: after `adb -s`,
#     `--target`, ANDROID_SERIAL= or "serial", and before `android
#     android-device` in a devices listing; a placeholder with a hyphen, such
#     as NEXUS5X-SERIAL or emulator-5554, passes;
#   - an Apple team id, ten capitals and digits, after a team's variable or
#     name or in a signing identity's parentheses; placeholders starting with
#     TEAM or PLACEHOLD pass;
#   - an email address, other than one under example.com, .org or .net,
#     GitHub's own support@ and noreply@ addresses, which a bot's sign-off
#     carries, and Kotlin's this@Class, and outside the third-party notices
#     and their template, which keep their authors' addresses as they wrote
#     them;
#   - a Mac's model identifier, as `sysctl -n hw.model` prints it.
#
# Write the model, the environment variable or a placeholder instead
# (IPHONE-SE-UDID, NEXUS5X-SERIAL, TEAMPLACE1, $HOME, $TMPDIR, <repo>). The
# values that only a pattern of their own would catch, such as names and
# handles, live in a private terms file outside the repository, which the
# scans read when it is present.
#
# This file spells the patterns out, so the scans leave it out. The
# expressions are single-quoted, their dollar signs literal.
# shellcheck shell=bash disable=SC2016

PUBLIC_NAMES=()
PUBLIC_REGEXES=()
# Per pattern, the files it leaves out, one path per line.
PUBLIC_SPARED=()

# public_pattern <name> <regex> [path...]: one pattern, and the files it
# leaves out.
public_pattern() {
  local name=$1 regex=$2
  shift 2
  PUBLIC_NAMES+=("$name")
  PUBLIC_REGEXES+=("$regex")
  PUBLIC_SPARED+=("$(printf '%s\n' "$@")")
}

public_pattern "a macOS home directory" \
  '/Users/(?!Shared(?![A-Za-z0-9._-]))[A-Za-z0-9_][A-Za-z0-9._-]*'
public_pattern "a Linux home directory" \
  '/home/(?!(?:me|user|runner)(?![A-Za-z0-9._-]))[A-Za-z0-9_][A-Za-z0-9._-]*'
public_pattern "Claude Code's per-user directory" \
  '(?:~|\$HOME|\$\{HOME\})/\.claude'
public_pattern "a Claude Code scratch directory" \
  '/tmp/claude-[0-9]+'
public_pattern "the macOS per-user temporary directory" \
  '/var/folders/'
public_pattern "the directory of this repository's worktrees" \
  'mobile-agent-wt'
public_pattern "an iOS device identifier" \
  '(?<![0-9A-Fa-f])0000[0-9A-Fa-f]{4}-[0-9A-Fa-f]{16}(?![0-9A-Fa-f])'
public_pattern "an iOS device identifier's prefix" \
  '(?<![0-9A-Fa-fxX-])0000[1-9][0-9A-Fa-f]{3}(?![0-9A-Fa-f-])'
public_pattern "an Android device serial" \
  '(?:\badb\s+-s\s+|--target[ =]+|ANDROID_SERIAL=|\bserial[ :=]+)[`'"'"'"]?(?=[0-9A-Za-z]*[0-9])[0-9A-Za-z]{8,20}(?![-0-9A-Za-z])|(?<![-0-9A-Za-z])(?=[0-9A-Za-z]*[0-9])[0-9A-Za-z]{8,20}\s+android\s+android-device\b'
public_pattern "an Apple team id" \
  '(?:DEVELOPMENT_TEAM|TEAM_ID|IOS_TEAM|TeamIdentifier|\b[Tt]eam(?: [Ii][Dd]| identifier)?)[`'"'"'"]?\s*[=:]?\s*[`'"'"'"(]?(?!TEAM|PLACEHOLD)(?=[A-Z0-9]*[0-9])(?=[A-Z0-9]*[A-Z])[A-Z0-9]{10}(?![A-Za-z0-9])|\((?!TEAM|PLACEHOLD)(?=[A-Z0-9]*[0-9])(?=[A-Z0-9]*[A-Z])[A-Z0-9]{10}\)'
public_pattern "an email address" \
  '(?<![A-Za-z0-9._%+-])(?!this@)(?!(?:support|noreply)@github\.com\b)[A-Za-z0-9._%+-]+@(?!example\.(?:com|org|net)\b)[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}' \
  THIRD-PARTY-NOTICES.md about.hbs
public_pattern "a Mac model identifier" \
  '\b(?:MacBookPro|MacBookAir|MacBook|Macmini|MacPro|iMacPro|iMac|MacStudio|Mac)[0-9]+,[0-9]+\b'

# The private terms file, one value per line, blank lines and lines starting
# with # left out: the file MOBILE_AGENT_PRIVATE_TERMS names, else the
# maintainer's copy outside the repository.
public_terms_file() {
  if [ -n "${MOBILE_AGENT_PRIVATE_TERMS:-}" ]; then
    printf '%s\n' "$MOBILE_AGENT_PRIVATE_TERMS"
  else
    printf '%s\n' "$HOME/code/mobile-agent-private/private-terms.txt"
  fi
}

# public_term_exempt <path> <text>: whether a line may hold a private term,
# which only the maintainer's name in LICENSE's copyright line and the
# owners' handles in CODEOWNERS may.
public_term_exempt() {
  case $1 in
    LICENSE) [[ $2 =~ ^Copyright\ \(c\)\ [0-9]{4}\  ]] ;;
    .github/CODEOWNERS) [[ $2 =~ ^[^#[:space:]][^[:space:]]*([[:space:]]+@[A-Za-z0-9][A-Za-z0-9/-]*)+[[:space:]]*$ ]] ;;
    *) return 1 ;;
  esac
}
