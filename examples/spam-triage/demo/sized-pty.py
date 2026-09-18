"""Run a command in a pty of a fixed size, streaming its output.

The headless recorder needs a terminal whose dimensions are known before the
program writes to it, so the captured cast is laid out the same on every
machine. Without this the recorder inherits whatever size the invoking shell
has — 80x24 when there is no tty at all — and the report wraps.
"""

import fcntl
import os
import pty
import struct
import sys
import termios

ROWS, COLS = 50, 110

if len(sys.argv) < 2:
    sys.exit("usage: sized-pty.py COMMAND [ARG...]")

pid, fd = pty.fork()
if pid == 0:
    # A short pause so the parent sizes the pty before the child reads it.
    os.execvp("sh", ["sh", "-c", 'sleep 0.4; exec "$@"', "sh"] + sys.argv[1:])

fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
while True:
    try:
        chunk = os.read(fd, 65536)
    except OSError:
        break
    if not chunk:
        break
    sys.stdout.buffer.write(chunk)
    sys.stdout.buffer.flush()

_, status = os.waitpid(pid, 0)
sys.exit(os.waitstatus_to_exitcode(status))
