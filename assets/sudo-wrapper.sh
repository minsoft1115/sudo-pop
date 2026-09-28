#!/bin/sh
# Installed by sudo-pop. Preserve sudo-pop's run0 routing and caller options.
exec @SUDO_POP_EXE@ "$@"
