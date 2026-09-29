#!/bin/sh
# Installed by sudo-pop. Default policy is run0-only; pass explicit policy settings through unchanged.
exec @SUDO_POP_EXE@ "$@"
