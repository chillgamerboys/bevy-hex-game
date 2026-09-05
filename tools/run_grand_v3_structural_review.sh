#!/usr/bin/env bash
set -euo pipefail

review_repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
review_mode="checkpoint"
if [[ "${1:-}" == "author" || "${1:-}" == "checkpoint" ]]; then
    review_mode="$1"
    shift
fi

# Compatibility entrypoint. The Python wrapper owns source provenance, free-space
# policy, advisory locking, dual incomplete markers, exact Cargo executable
# resolution, active toolchain evidence, strict-vs-draft state, timings, and
# fail-closed artifact publication.
# Existing callers that pass only --seed/--output retain the
# release/nonincremental checkpoint behavior.
cd "${review_repository}"
exec python3 tools/review.py structural "${review_mode}" "$@"
