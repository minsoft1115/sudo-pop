# Installed by `sudo-pop --init`.
#
# Aliases only expand in interactive shells, so scripts, `sh -c`, Makefiles and
# systemd units keep using the real sudo. Run /usr/bin/sudo to bypass this on
# purpose: an absolute path goes around aliases, shell functions and PATH alike.
alias sudo='sudo-pop'
