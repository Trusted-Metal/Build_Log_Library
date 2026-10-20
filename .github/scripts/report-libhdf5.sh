#!/usr/bin/env bash
# Reports the libhdf5 that one CI test leg linked, and checks it (IP-01 phase 1I, spike S5; requirement BL-ST-01).
#
#   report-libhdf5.sh <leg name> [--exactly X.Y.Z]
#
# Environment:
#   LIBHDF5_MINIMUM      required: the candidate minimum libhdf5. The leg fails if the linked version is older.
#   LIBHDF5_CARGO_ARGS   optional: extra cargo arguments. CI leaves it empty; a machine without libhdf5 passes `--features static-hdf5`.
#   LIBHDF5_REPORT_DIR   optional: where the report goes (default `libhdf5-report`, which the workflow uploads as an artifact).
#
# It writes `libhdf5-version.txt` into the report folder, adds the version to the job summary, and exits 1 if a check failed. The version is
# the one the library itself reports at run time (`H5get_libversion`), so on Windows it is the version of the hdf5.dll that was loaded.
set -u

leg="${1:?usage: report-libhdf5.sh <leg name> [--exactly X.Y.Z]}"
shift
minimum="${LIBHDF5_MINIMUM:?LIBHDF5_MINIMUM must name the minimum libhdf5, for example 1.10.8}"
dir="${LIBHDF5_REPORT_DIR:-libhdf5-report}"
mkdir -p "$dir"

checks=(--at-least "$minimum" "$@")

# shellcheck disable=SC2086  # the extra arguments are meant to split into words
version="$(cargo run --locked --quiet -p build-log-io ${LIBHDF5_CARGO_ARGS:-} --example libhdf5_version -- "${checks[@]}")"
status=$?

if [ "$status" -eq 0 ]; then verdict="passed"; else verdict="FAILED"; fi

{
  echo "leg: $leg"
  echo "libhdf5: ${version:-unknown}"
  echo "minimum: $minimum"
  echo "checks: ${checks[*]}"
  echo "result: $verdict"
} > "$dir/libhdf5-version.txt"
cat "$dir/libhdf5-version.txt"

{
  echo "### libhdf5 on $leg"
  echo
  echo "**${version:-unknown}** (minimum $minimum): $verdict"
  if [ -f "$dir/install.txt" ]; then
    echo
    echo '```text'
    cat "$dir/install.txt"
    echo '```'
  fi
} >> "${GITHUB_STEP_SUMMARY:-/dev/null}"

exit "$status"
