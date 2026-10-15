#!/usr/bin/env bash
# The publish scan over a repository's whole history, for a rewritten clone
# before it is pushed (task 5.1 of the open-sourcing change). It checks:
#
#   - every blob of every commit a ref reaches against the patterns of
#     scripts/public-patterns.sh, which scripts/check-public.sh applies to the
#     tracked files, with the same files spared; lock files, binary blobs and
#     the scripts that spell the patterns out (public-patterns.sh, and
#     check-local-paths.sh before it) are left out of the patterns;
#   - the same blobs, in any case, against the values of the private terms
#     file when it is present (MOBILE_AGENT_PRIVATE_TERMS names it, or else it
#     is the maintainer's copy outside the repository), LICENSE's copyright
#     line and CODEOWNERS' owners aside;
#   - every path a commit holds, and every commit message, against the
#     patterns and the private terms;
#   - every commit with gitleaks, when it is installed, with .gitleaks.toml.
#
# Usage: scripts/check-history.sh [<git rev-list arguments>]
#
# The arguments choose the commits, every ref's by default (--all). Prints
# each finding in a blob as path:line: what it is (blob <id>, from commit
# <id>), in a message as <commit>: what it is in the message, a private term
# named by its line in the list, never by its value, and a summary. Exits 1
# when there is a finding, 0 when there is none, 2 when the private terms
# file it was pointed at is missing, and with git's status when git fails.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/public-patterns.sh
. "$here/public-patterns.sh"

root=$(git rev-parse --show-toplevel)
cd "$root"
if [ "$#" -eq 0 ]; then set -- --all; fi

scratch="$(mktemp -d "${TMPDIR:-/tmp}/check-history.XXXXXX")"
trap 'rm -rf "${scratch:?}"' EXIT
findings="$scratch/findings"
: > "$findings"

# The patterns, one per line: name, expression and the paths it spares,
# separated by tabs.
for i in "${!PUBLIC_REGEXES[@]}"; do
  spared=$(printf '%s' "${PUBLIC_SPARED[$i]}" | tr '\n' '\t')
  printf '%s\t%s\t%s\n' "${PUBLIC_NAMES[$i]}" "${PUBLIC_REGEXES[$i]}" "$spared"
done > "$scratch/patterns"

terms=$(public_terms_file)
shown=${terms/#"$HOME"/\$HOME}
if [ -f "$terms" ]; then
  cp "$terms" "$scratch/terms"
elif [ -n "${MOBILE_AGENT_PRIVATE_TERMS:-}" ]; then
  echo "check-history: MOBILE_AGENT_PRIVATE_TERMS names $terms, which does not exist" >&2
  exit 2
else
  echo "check-history: no private terms file at $shown; checking the patterns alone"
  : > "$scratch/terms"
fi

# Every blob the commits reach, once, with the first path it was seen at.
git rev-list --objects "$@" > "$scratch/objects"
git cat-file --batch-check='%(objecttype) %(objectname) %(rest)' < "$scratch/objects" \
  | awk '$1 == "blob" { sub(/^blob /, ""); print }' | sort -u -k1,1 > "$scratch/blobs"
commits=$(git rev-list --count "$@")
blobs=$(wc -l < "$scratch/blobs" | tr -d ' ')
cut -d ' ' -f 1 "$scratch/blobs" > "$scratch/blob-ids"

# The blobs' text and the commit messages, through the same rules.
scan_text() {
  perl -e '
    use strict; use warnings;
    my ($patterns, $terms, $shown, $mode, $paths) = @ARGV;
    my @patterns;
    open my $p, "<", $patterns or die;
    while (<$p>) {
      chomp;
      my ($name, $regex, @spared) = split /\t/;
      push @patterns, [$name, qr/$regex/, { map { $_ => 1 } grep { length } @spared }];
    }
    my @terms;
    open my $t, "<", $terms or die;
    my $number = 0;
    while (my $line = <$t>) {
      $number++;
      $line =~ s/\r?\n\z//;
      next if $line eq "" || $line =~ /^#/;
      push @terms, [$number, lc $line];
    }
    my %path;
    if ($mode eq "blobs") {
      open my $b, "<", $paths or die;
      while (<$b>) { chomp; my ($id, $at) = split / /, $_, 2; $path{$id} = $at // ""; }
    }
    my $left_out = qr{(?:\.lock\z|\Ascripts/public-patterns\.sh\z|\Ascripts/check-local-paths\.sh\z)};
    sub exempt {
      my ($at, $text) = @_;
      return $text =~ /^Copyright \(c\) [0-9]{4} / if $at eq "LICENSE";
      return $text =~ m{^[^#\s]\S*(\s+@[A-Za-z0-9][A-Za-z0-9/-]*)+\s*$} if $at eq ".github/CODEOWNERS";
      return 0;
    }
    sub check {
      my ($where, $at, $data, $patterned) = @_;
      my @lines = split /\n/, $data, -1;
      for my $n (0 .. $#lines) {
        my $text = $lines[$n];
        my $label = $mode eq "blobs" ? sprintf("%s:%d: ", $at, $n + 1) : "$where: ";
        my $suffix = $mode eq "blobs" ? " (blob $where)" : " in the message";
        if ($patterned) {
          for my $pattern (@patterns) {
            my ($name, $regex, $spared) = @$pattern;
            next if $spared->{$at};
            print "$label$name$suffix\n" if $text =~ $regex;
          }
        }
        my $lower = lc $text;
        for my $term (@terms) {
          next if index($lower, $term->[1]) < 0;
          next if exempt($at, $text);
          print "${label}private term on line $term->[0] of $shown$suffix\n";
        }
      }
    }
    binmode STDIN;
    if ($mode eq "names") {
      while (my $at = <STDIN>) {
        chomp $at;
        for my $pattern (@patterns) {
          my ($name, $regex) = @$pattern;
          print "$at: $name in the file\x27s name\n" if $at =~ $regex;
        }
        my $lower = lc $at;
        for my $term (@terms) {
          print "$at: private term on line $term->[0] of $shown in the file\x27s name\n"
            if index($lower, $term->[1]) >= 0;
        }
      }
    } elsif ($mode eq "blobs") {
      while (my $header = <STDIN>) {
        my ($id, $type, $size) = split / /, $header;
        my $data = "";
        read(STDIN, $data, $size) == $size or die "short read of $id";
        <STDIN>;
        my $at = $path{$id} // "";
        my $binary = index(substr($data, 0, 8000), "\0") >= 0;
        check($id, $at, $data, !$binary && $at !~ $left_out);
      }
    } else {
      local $/ = "\x01";
      while (my $record = <STDIN>) {
        chomp $record;
        $record =~ s/\A\n//;
        my ($id, $message) = split /\x00/, $record, 2;
        next unless defined $message;
        check(substr($id, 0, 12), "", $message, 1);
      }
    }
  ' "$scratch/patterns" "$scratch/terms" "$shown" "$@"
}

git cat-file --batch < "$scratch/blob-ids" | scan_text blobs "$scratch/blobs" > "$scratch/blob-findings"
# Every path a commit added, renames read as an addition, so every name a
# file has had.
git log --format= --name-only --no-renames "$@" | sed '/^$/d' | sort -u | scan_text names \
  > "$scratch/name-findings"
git log --format='%H%x00%B%x01' "$@" | scan_text messages > "$scratch/message-findings"

# Each blob finding with the commit that first brought its blob.
sed -e 's/.*(blob \([0-9a-f]*\))$/\1/' "$scratch/blob-findings" | sort -u > "$scratch/found-blobs"
while IFS= read -r blob; do
  printf '%s %s\n' "$blob" "$(git log --format=%h --reverse --find-object="$blob" "$@" | head -n 1)"
done < "$scratch/found-blobs" > "$scratch/first-commits"
awk 'NR == FNR { first[$1] = $2; next }
     { blob = $0; sub(/.*\(blob /, "", blob); sub(/\)$/, "", blob);
       line = $0; sub(/\)$/, "", line); print line ", from commit " first[blob] ")" }' \
  "$scratch/first-commits" "$scratch/blob-findings" >> "$findings"
cat "$scratch/name-findings" "$scratch/message-findings" >> "$findings"

# gitleaks over every commit.
if command -v gitleaks > /dev/null 2>&1; then
  printf '%s\n' '{{range .}}gitleaks: {{.RuleID}} at {{.File}}:{{.StartLine}} in {{.Commit}}' '{{end}}' \
    > "$scratch/report.tmpl"
  config=()
  if [ -f "$root/.gitleaks.toml" ]; then config=(--config "$root/.gitleaks.toml"); fi
  status=0
  gitleaks git --no-banner --log-level error --redact ${config[@]+"${config[@]}"} \
    --log-opts "$*" --report-format template --report-template "$scratch/report.tmpl" \
    --report-path "$scratch/gitleaks" "$root" || status=$?
  if [ "$status" -gt 1 ]; then
    echo "check-history: gitleaks failed with status $status" >&2
    exit "$status"
  fi
  sed -e '/^$/d' "$scratch/gitleaks" >> "$findings"
else
  echo "check-history: gitleaks is not installed; skipping its rules"
fi

if [ "$commits" = 1 ]; then counted="1 commit"; else counted="$commits commits"; fi
if [ -s "$findings" ]; then
  cat "$findings"
  count=$(wc -l < "$findings" | tr -d ' ')
  if [ "$count" = 1 ]; then noun=finding; else noun=findings; fi
  echo "check-history: $count $noun in $counted and $blobs blobs" >&2
  exit 1
fi
echo "check-history: $counted and $blobs blobs: no machine path, device identifier, team id, email, host model, private term or credential"
