#!/usr/bin/env bash
# The former name of the publish scan, which grew from this check of
# machine-local paths; it runs scripts/check-public.sh with its arguments.
exec "$(dirname "${BASH_SOURCE[0]}")/check-public.sh" "$@"
