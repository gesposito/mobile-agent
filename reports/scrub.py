#!/usr/bin/env python3
"""Rewrites the reports' provenance and samplers for a reader of the public
repository, as task 2.1 of the `open-sourcing` change did.

- A host is its chip, cores, memory and macOS version: the machine's model
  identifier in parentheses after the chip goes, so `Apple M1 Max (<model>)`
  becomes `Apple M1 Max`, in prose, Markdown rows and JSON alike.
- A sampler line (`<time> load1=... busy=[...] top=[<name> <share>%, ...]`)
  keeps its loads, its launch time, its busy list and its CPU shares; its top
  field loses the process names.
- The endpoint security product is a value of the private terms file that the
  reports themselves show to be the product: in the parenthesis after
  "endpoint security" or within a sampler's process name. It leaves that
  parenthesis and becomes "the endpoint security software" elsewhere.

A changed line keeps every number but those of the model identifier and the
process names it drops, and a file that would keep a private term is refused,
the term named by its line in the list and never by its value. Nothing is
written while anything is refused.

Usage: python3 reports/scrub.py [--terms FILE] [--check] [PATH...]

PATH defaults to every tracked file under reports/. The terms come from
--terms, else the file MOBILE_AGENT_PRIVATE_TERMS names, else
$HOME/code/mobile-agent-private/private-terms.txt when it exists: one value
per line, blank lines and lines starting with `#` left out. --check writes
nothing and lists the files a scrub would change. Exits 0 when done or with
nothing to do, 1 on a refusal or, with --check, when a scrub is due, and 2 on
a usage error.
"""

import argparse
import collections
import os
import re
import subprocess
import sys

NEUTRAL = "the endpoint security software"

# A Mac's model identifier in parentheses, as `sysctl -n hw.model` prints it.
MODEL = re.compile(
    r" \((?:MacBookPro|MacBookAir|MacBook|Macmini|MacPro|iMacPro|iMac|MacStudio|Mac)\d+,\d+\)"
)

# A sampler line and its top field, the line's last.
SAMPLER = re.compile(
    r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z load1=\S+ .*\btop=\[)(.*)(\]\s*)$", re.S
)
SHARE = r"\d+(?:\.\d+)?%"
ENTRY = re.compile(r"(.+?) (" + SHARE + r")(?:, |$)")
SHARES_ONLY = re.compile(r"^(?:" + SHARE + r"(?:, " + SHARE + r")*)?$")
NUMBER = re.compile(r"\d+(?:\.\d+)?")


def terms_from(path):
    """The private terms of `path` as (line number, value) pairs."""
    terms = []
    with open(path, encoding="utf-8") as listing:
        for number, line in enumerate(listing, 1):
            value = line.strip()
            if value and not value.startswith("#"):
                terms.append((number, value))
    return terms


def terms_path(given):
    if given:
        return given
    named = os.environ.get("MOBILE_AGENT_PRIVATE_TERMS")
    if named:
        return named
    default = os.path.join(
        os.environ.get("HOME", ""), "code", "mobile-agent-private", "private-terms.txt"
    )
    return default if os.path.isfile(default) else None


def top_entries(inner):
    """The (name, share) entries of a top field, or None when it does not read."""
    entries, at = [], 0
    for match in ENTRY.finditer(inner):
        if match.start() != at:
            return None
        entries.append((match.group(1), match.group(2)))
        at = match.end()
    return entries if at == len(inner) else None


def sampler(line):
    """A sampler line without the process names, and the names it dropped, or
    None when `line` is no sampler line or already has none."""
    match = SAMPLER.match(line)
    if not match:
        return None
    head, inner, tail = match.groups()
    if SHARES_ONLY.match(inner):
        return None
    entries = top_entries(inner)
    if entries is None:
        raise ValueError("a top field that does not read as names and shares")
    shares = ", ".join(share for _, share in entries)
    return head + shares + tail, [name for name, _ in entries]


def process_names(text):
    """The process names of the sampler lines of `text`."""
    names = []
    for line in text.splitlines():
        try:
            names += (sampler(line) or ("", []))[1]
        except ValueError:
            continue
    return names


def product_terms(texts, terms):
    """The terms the texts show to be the endpoint security product."""
    names = [name.lower() for text in texts for name in process_names(text)]
    found = []
    for number, value in terms:
        named = re.compile(r"endpoint security(?: extension)? \(" + re.escape(value) + r"\)")
        lowered = value.lower()
        if any(named.search(text) for text in texts) or any(lowered in n for n in names):
            found.append((number, value))
    return found


def prose(line, products):
    """`line` with each product term worded neutrally."""
    for _, value in products:
        term = re.escape(value)
        line = re.sub(r"(endpoint security(?: extension)?) \(" + term + r"\)", r"\1", line)
        line = re.sub(
            r"(^\s*[-*]\s+|^|[.!?]\s+)" + term + r"(?![\w-])",
            lambda m: m.group(1) + NEUTRAL[0].upper() + NEUTRAL[1:],
            line,
        )
        line = re.sub(r"(?<![\w.-])" + term + r"(?![\w-])", NEUTRAL, line)
    return line


def scrub_line(line, products):
    """`line` scrubbed, and the text it dropped."""
    dropped = MODEL.findall(line)
    line = MODEL.sub("", line)
    rewritten = sampler(line)
    if rewritten:
        line, names = rewritten
        dropped += names
    else:
        line = prose(line, products)
    return line, dropped


def numbers(text):
    return collections.Counter(NUMBER.findall(text))


def main(argv):
    parser = argparse.ArgumentParser(
        description="Rewrites the reports' provenance and samplers for a public reader."
    )
    parser.add_argument("--terms", help="the private terms file")
    parser.add_argument("--check", action="store_true", help="write nothing; exit 1 when a scrub is due")
    parser.add_argument("paths", nargs="*")
    args = parser.parse_args(argv)

    here = os.path.dirname(os.path.abspath(__file__))
    paths = args.paths
    if not paths:
        root = os.path.dirname(here)
        listed = subprocess.run(
            ["git", "-C", root, "ls-files", "-z", "reports"],
            check=True, capture_output=True,
        ).stdout.decode("utf-8")
        paths = [
            os.path.join(root, p) for p in listed.split("\0")
            if p and os.path.abspath(os.path.join(root, p)) != os.path.abspath(__file__)
        ]

    source = terms_path(args.terms)
    terms = terms_from(source) if source else []

    texts = {}
    for path in paths:
        try:
            with open(path, encoding="utf-8", newline="") as f:
                texts[path] = f.read()
        except (UnicodeDecodeError, IsADirectoryError):
            continue
    products = product_terms(texts.values(), terms)

    refusals, changed = [], {}
    for path, text in texts.items():
        old_lines = text.splitlines(keepends=True)
        new_lines = []
        for number, old in enumerate(old_lines, 1):
            try:
                new, dropped = scrub_line(old, products)
            except ValueError as error:
                refusals.append("%s:%d: %s" % (path, number, error))
                new, dropped = old, []
            if numbers(old) - numbers(" ".join(dropped)) != numbers(new):
                refusals.append("%s:%d: the scrub would change a number" % (path, number))
            for term_line, value in terms:
                if value.lower() in new.lower():
                    refusals.append(
                        "%s:%d: private term on line %d of %s" % (path, number, term_line, source)
                    )
            new_lines.append(new)
        if new_lines != old_lines:
            changed[path] = (sum(a != b for a, b in zip(old_lines, new_lines)), "".join(new_lines))

    if refusals:
        for refusal in refusals:
            print(refusal, file=sys.stderr)
        print("scrub: %d refusal(s); nothing written" % len(refusals), file=sys.stderr)
        return 1
    for path, (lines, text) in sorted(changed.items()):
        shown = os.path.relpath(path)
        shown = path if shown.startswith("..") else shown
        if args.check:
            print("would scrub %s (%s)" % (shown, count(lines, "line")))
        else:
            with open(path, "w", encoding="utf-8", newline="") as f:
                f.write(text)
            print("scrubbed %s (%s)" % (shown, count(lines, "line")))
    total = sum(lines for lines, _ in changed.values())
    print(
        "scrub: %s, %s %s, every other number kept"
        % (count(len(changed), "file"), count(total, "line"), "to change" if args.check else "changed")
    )
    return 1 if args.check and changed else 0


def count(n, noun):
    return "%d %s%s" % (n, noun, "" if n == 1 else "s")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
