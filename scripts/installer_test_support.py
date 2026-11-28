"""Terminal capture shared by the offline installer tests."""

from contextlib import contextmanager
import errno
import os
import threading


@contextmanager
def captured_terminal(enabled):
    """Yield the child FD and a buffer whose contents are complete on exit."""
    if not enabled:
        yield None, b""
        return

    master, slave = os.openpty()
    output = bytearray()
    errors = []

    def drain():
        try:
            while chunk := os.read(master, 4096):
                output.extend(chunk)
        except OSError as error:
            # Linux reports EIO when the slave closes. Other systems return EOF.
            if error.errno != errno.EIO:
                errors.append(error)

    reader = threading.Thread(target=drain, daemon=True)
    try:
        try:
            reader.start()
            yield slave, output
        finally:
            os.close(slave)
            if reader.ident is not None:
                reader.join(timeout=5)
        if reader.is_alive():
            raise AssertionError("PTY reader did not finish")
        if errors:
            raise errors[0]
    finally:
        os.close(master)
