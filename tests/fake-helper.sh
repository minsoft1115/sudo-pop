#!/usr/bin/env bash
#
# Stands in for polkit-agent-helper-1 so the conversation can be tested without
# a PAM stack, a password, or root. Speaks the same line protocol:
#
#   argv[1] is the username, the cookie arrives on stdin, prompts go out on
#   stdout, answers come back on stdin, and the last line is SUCCESS or FAILURE.
#
# FAKE_HELPER_MODE picks the scenario. FAKE_HELPER_LOG, if set, records what we
# were told, so a test can assert the cookie and username actually arrived.
set -u
mode="${FAKE_HELPER_MODE:-success}"
user="${1:-}"
read -r cookie || cookie=""
[ -n "${FAKE_HELPER_LOG:-}" ] && printf 'user=%s cookie=%s\n' "$user" "$cookie" >>"$FAKE_HELPER_LOG"

ask() { printf 'PAM_PROMPT_ECHO_OFF %s\n' "$1"; read -r answer || answer=""; }

case "$mode" in
  success)      ask "Password:"; echo SUCCESS ;;
  wrong)        ask "Password:"; echo FAILURE ;;
  # A locked account: the helper refuses before asking anything.
  no-prompt)    echo FAILURE ;;
  # The socket helper on a kernel without SO_PEERPIDFD: closes, says nothing.
  silent)       exit 0 ;;
  echo-on)      printf 'PAM_PROMPT_ECHO_ON Username:\n'; read -r answer || true; echo SUCCESS ;;
  info)         printf 'PAM_TEXT_INFO Place your finger\n'; ask "Password:"; echo SUCCESS ;;
  error-then-ok) printf 'PAM_ERROR_MSG Try again\n'; ask "Password:"; echo SUCCESS ;;
  # Answers only to one specific password, so a test can drive both outcomes.
  check)        ask "Password:"
                if [ "$answer" = "${FAKE_HELPER_PASSWORD:-open-sesame}" ]; then echo SUCCESS; else echo FAILURE; fi ;;
  *)            echo "FAILURE"; exit 1 ;;
esac
