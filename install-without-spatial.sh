#!/usr/bin/env bash
# Lynapse 2.0 without Spatial Audio (the Spatial Audio switch shows as unavailable). Extra flags pass through.
exec "$(dirname "$0")/install.sh" --no-spatial "$@"
