#!/usr/bin/env bash
# Builds every reference fixture: runs each fixtures/<name>/build.sh from its
# own directory, in name order, and after it, for the fixtures that also build
# for a physical iPhone (expo and swiftui), the same script with
# DESTINATION=device. A device build needs MOBILE_AGENT_IOS_TEAM; without it
# the script exits with status 3, and the build is reported as skipped. Every
# script runs even when an earlier one fails, and the exit status is non-zero
# if any of them failed.
set -euo pipefail

cd "$(dirname "$0")"
shopt -s nullglob
scripts=(*/build.sh)
device_builds=(expo swiftui)

# FIXTURES=name,name selects a subset (FIXTURES=none selects nothing); unset builds all.
if [ -n "${FIXTURES:-}" ]; then
  selected=()
  IFS=',' read -r -a wanted <<< "$FIXTURES"
  for name in "${wanted[@]}"; do
    [ "$name" = "none" ] && continue
    if [ -f "$name/build.sh" ]; then
      selected+=("$name/build.sh")
    else
      echo "fixtures/$name/build.sh not present, skipping"
    fi
  done
  scripts=(${selected[@]+"${selected[@]}"})
fi

if [ "${#scripts[@]}" -eq 0 ]; then
  echo "fixtures/*/build.sh not present, skipping"
  exit 0
fi

failed=()
for script in "${scripts[@]}"; do
  fixture=${script%/build.sh}
  echo "==> fixtures/$script"
  if ! (cd "$fixture" && ./build.sh); then
    echo "fixtures/$script failed" >&2
    failed+=("$fixture")
  fi
  if [[ " ${device_builds[*]} " == *" $fixture "* ]]; then
    echo "==> DESTINATION=device fixtures/$script"
    status=0
    (cd "$fixture" && DESTINATION=device ./build.sh) || status=$?
    case $status in
      0) ;;
      3) echo "fixtures/$script: DESTINATION=device needs MOBILE_AGENT_IOS_TEAM, skipping the device build" ;;
      *)
        echo "fixtures/$script with DESTINATION=device failed" >&2
        failed+=("$fixture (device)")
        ;;
    esac
  fi
done

if [ "${#failed[@]}" -gt 0 ]; then
  echo "fixture builds failed: ${failed[*]}" >&2
  exit 1
fi
