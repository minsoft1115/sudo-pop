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
  # Fingerprint success: a notice, then SUCCESS, never a password prompt.
  finger-ok)    printf 'PAM_TEXT_INFO Place your finger\n'; echo SUCCESS ;;
  # One failed swipe, then a match, still no password.
  finger-retry) printf 'PAM_TEXT_INFO Place your finger\n'
                printf 'PAM_ERROR_MSG Verification failed\n'
                printf 'PAM_TEXT_INFO Place your finger\n'
                echo SUCCESS ;;
  # Fingerprint gives up, then the password is always rejected.
  finger-then-fail)
    printf 'PAM_TEXT_INFO Place your finger\n'
    printf 'PAM_ERROR_MSG Verification failed\n'
    ask "Password:"
    echo FAILURE ;;
  # Fingerprint gives up, then PAM asks for a password.
  finger-then-pw) printf 'PAM_TEXT_INFO Place your finger\n'
                printf 'PAM_ERROR_MSG Verification failed\n'
                printf 'PAM_ERROR_MSG Verification failed\n'
                printf 'PAM_ERROR_MSG Verification failed\n'
                ask "Password:"; echo SUCCESS ;;
  # Notice, then silence, until stdin closes or we are killed. `exec` flushes
  # the line so the parent sees it before we hang. stdout stays open and
  # quiet, like pam_fprintd at the reader: redirecting it would hang up the
  # pipe and turn the wait into an EOF, which is not the case being modelled.
  finger-hang)  printf 'PAM_TEXT_INFO Place your finger\n'
                exec cat ;;
  # Two lines in one write, then a wait for the answer. The reader must not
  # poll the descriptor for a line it already holds in its buffer.
  burst)        printf 'PAM_TEXT_INFO Place your finger\nPAM_PROMPT_ECHO_OFF Password:\n'
                read -r answer || answer=""; echo SUCCESS ;;
  # Closes its stdin before asking, then lingers: the answer has nowhere to
  # go (EPIPE). That is a helper dying mid-conversation, not a wrong password.
  prompt-then-die) exec 0<&-
                printf 'PAM_PROMPT_ECHO_OFF Password:\n'; sleep 5 ;;
  # Answers only to one specific password, so a test can drive both outcomes.
  check)        ask "Password:"
                if [ "$answer" = "${FAKE_HELPER_PASSWORD:-open-sesame}" ]; then echo SUCCESS; else echo FAILURE; fi ;;
  *)            echo "FAILURE"; exit 1 ;;
esac
