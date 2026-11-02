"""Forward IMAP connections to a local server, resetting the first few mid-transfer.

Used by the native fault walkthrough. A connection is reset once the server has sent it more
than the given number of bytes, which is past the TLS handshake and inside a window's data. A
reset is an abrupt TCP reset (RST), which is how a network drop looks to the client. Traffic
never leaves loopback and carries only synthetic mail.

Arguments: listen port, target port, log file, maximum resets, bytes from the server before a
reset. The log records each accepted connection as "accept" and each reset as "reset".
"""

import socket
import struct
import sys
import threading

listen_port, target_port = int(sys.argv[1]), int(sys.argv[2])
log_path, max_resets, threshold = sys.argv[3], int(sys.argv[4]), int(sys.argv[5])
lock = threading.Lock()
resets = 0


def note(text):
    with open(log_path, "a", encoding="utf-8") as log:
        log.write(text + "\n")


def reset(*sockets):
    for sock in sockets:
        try:
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
            sock.close()
        except OSError:
            pass


def pump(source, destination, forwarded=None):
    received = 0
    try:
        while True:
            data = source.recv(65536)
            if not data:
                break
            destination.sendall(data)
            received += len(data)
            if forwarded is not None:
                forwarded(received)
    except OSError:
        pass
    finally:
        try:
            destination.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass


def serve(client):
    global resets
    try:
        upstream = socket.create_connection(("127.0.0.1", target_port))
    except OSError:
        client.close()
        return
    reset_here = False

    def maybe_reset(received):
        nonlocal reset_here
        global resets
        if reset_here or received < threshold:
            return
        with lock:
            if resets >= max_resets:
                return
            resets += 1
            reset_here = True
            note(f"reset {resets} after {received} bytes")
        reset(client, upstream)

    threading.Thread(target=pump, args=(client, upstream), daemon=True).start()
    pump(upstream, client, maybe_reset)
    reset(client, upstream)


def main():
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", listen_port))
    server.listen(16)
    note("ready")
    while True:
        client, _ = server.accept()
        note("accept")
        threading.Thread(target=serve, args=(client,), daemon=True).start()


if __name__ == "__main__":
    main()
