#!/usr/bin/env bash
# Lynapse 2.0 + Spatial Audio (virtual surround filter with the free KEMAR HRIR). Extra flags pass through (see install.sh --help).
exec "$(dirname "$0")/install.sh" --spatial "$@"
