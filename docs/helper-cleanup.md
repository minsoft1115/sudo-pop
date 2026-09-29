# Cancelled socket helper cleanup

A socket-activated `polkit-agent-helper-1` can remain inside `pam_fprintd`
after its prompt exits. Closing the client socket does not synchronously cancel
PAM. Starting a second helper during that interval can fail to claim the sensor
and fall through to a password prompt.

## Ownership and ordering

1. The agent and each prompt share a private Unix socket on the prompt's stdin.
   The cookie still travels privately, never in argv or environment variables.
2. Before connecting a helper, the prompt requests the agent's cleanup gate.
   While another helper is finishing, the window displays
   `Finishing previous authentication…`, hides authentication input, and permits
   cancellation. The caller's existing deadline is not extended.
3. The prompt connects the helper itself, preserving the prompt PID as the
   helper's peer identity. Before sending the username or cookie it transfers a
   duplicate socket descriptor to the agent with `SCM_RIGHTS` and waits for an
   acknowledgement. Unconfirmed tracking stops authentication.
4. A finished attempt releases its ticket. The agent shuts down the socket's
   write direction, discards subsequent helper output, and waits for read EOF.
   A late `SUCCESS` or `FAILURE` line does not release the gate.
5. Closing the window returns cancellation promptly. The prompt exits and the
   agent returns its normal polkit response without waiting for PAM cleanup.
   The background monitor holds the gate independently. Control-socket EOF also
   triggers cleanup if the prompt crashes or is killed without a destructor.
6. Only confirmed EOF releases the gate for the next helper. A socket-to-fork
   fallback and a retry within the same window must reacquire it too.

The agent does not read the live PAM conversation or receive password answers
through the control protocol. It holds a socket descriptor during authentication
and reads/discards helper output only during cleanup. The normal authentication
result remains the helper's response to polkit, not a parsed cleanup message.

## Limits

- This does not immediately stop the root helper or reset the sensor. It avoids
  overlapping our requests while the previous helper finishes naturally.
- Half-closing a socket is not authentication revocation. Cancellation must
  still return promptly to polkit, and the prompt process must exit. Retaining
  the descriptor does not keep the prompt process alive; the helper's peer
  pidfd continues to identify that prompt, not the agent.
- A waiting request stops after 30 seconds at the latest (or its earlier caller
  cancellation/deadline). The background monitor keeps waiting. Time alone never
  permits a new helper. Cleanup errors without EOF block further authentication
  in that agent instance rather than assuming the sensor was released.
- EOF confirms this helper connection finished. It cannot guarantee that a
  different application has not claimed the device, or that a broken backend
  completed device cleanup. Other applications and agent restarts are outside
  this in-memory gate; restarting the agent does not stop old root helpers.
- Forked setuid helpers retain their existing kill/reap path. No new root
  privilege, polkit policy, PAM change, or service restart is introduced.

## Verification

Automated tests cover descriptor ownership/CLOEXEC, cancellation with a lingering
socket helper, rejection of late success as completion, prompt death without a
release message, cancellation while waiting, retry on the same control socket,
fail-closed behavior, and GUI waiting/input/deadline behavior. They use fake
helpers and never authenticate against the host PAM stack.

`tests/prompt-driver.py` supplies the new private socket protocol to standalone
GUI scenarios while preserving the prompt PID. The production agent owns the
actual cleanup implementation in `src/cleanup.rs`.

Manual checks after installation:

1. Start `sudo true`, cancel while waiting for a fingerprint, and immediately
   repeat. The second window should show cleanup status until the first helper
   closes, then start a fresh fingerprint request.
2. Cancel the waiting second request: its window must close promptly, and it
   must never start a helper later after the first request finishes.
3. Let the caller time out during fingerprint waiting, then repeat. The same
   cleanup gate must apply; waiting must not restart the caller's timeout.
4. After cleanup, verify fingerprint success and password fallback normally.
   Also check that typing a password during a cleanup notice is not accepted.
