#!/usr/bin/env python3
"""Launch a standalone prompt with the agent's private socket protocol.

The original process execs the prompt, preserving $! for GUI assertions. A small
supervisor retains transferred helper sockets and discards cleanup output.
Production uses src/cleanup.rs; this driver is only for standalone UI scenarios.
"""
import array
import os
import socket
import sys

cookie = sys.stdin.buffer.readline()
client, server = socket.socketpair()
if os.fork() == 0:
    client.close()
    helper = None
    try:
        server.sendall(cookie)
        while True:
            command, ancillary, flags, _ = server.recvmsg(1, socket.CMSG_SPACE(4))
            if command == b"B":
                server.sendall(b"R")
            elif command == b"T":
                assert helper is None and not flags & socket.MSG_CTRUNC
                fds = array.array("i")
                for level, kind, data in ancillary:
                    if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
                        fds.frombytes(data)
                assert len(fds) == 1
                helper = socket.socket(fileno=fds[0])
                server.sendall(b"R")
            elif command == b"D" or not command:
                if helper is not None:
                    helper.shutdown(socket.SHUT_WR)
                    while helper.recv(4096):
                        pass
                    helper.close()
                    helper = None
                if not command:
                    break
            else:
                raise RuntimeError("invalid prompt control message")
    except (OSError, AssertionError, RuntimeError) as error:
        print(f"prompt-driver: {error}", file=sys.stderr)
    finally:
        if helper is not None:
            helper.close()
        server.close()
    os._exit(0)
server.close()
os.dup2(client.fileno(), 0)
client.close()
os.execv(sys.argv[1], sys.argv[1:])
