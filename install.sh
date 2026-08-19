#!/usr/bin/env bash
#
# The implementation moved to old/ while the polkit agent is being written
# (docs/polkit-agent.md). Installing still means installing that one, so this
# forwards rather than making every caller learn the new path -- omarchy-setup
# calls this file by name.
#
# Delete this file once the new build replaces it.
exec bash "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/old/install.sh" "$@"
