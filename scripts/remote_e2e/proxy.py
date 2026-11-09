#!/usr/bin/env python3
"""TLS termination and bounded, controllable TCP faults. Never logs payloads."""
import argparse
import select
import socket
import ssl
import struct
import threading
import time


class Proxy:
    def __init__(self, upstream, *, port=0, context=None, reject_websocket=False, inspect_http=False):
        self.upstream, self.context = upstream, context
        self.delay = 0
        self.bytes_per_second = 0
        self.blackhole = False
        self.drop_responses = False
        self.reject_websocket = reject_websocket
        self.inspect_http = inspect_http or reject_websocket
        self.request_counts = {}
        self.closed = threading.Event()
        self.listener = socket.socket()
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind(("127.0.0.1", port))
        self.listener.listen(16)
        self.listener.settimeout(0.2)
        self.port = self.listener.getsockname()[1]
        self.connections = set()
        self.lock = threading.Lock()
        self.thread = threading.Thread(target=self.accept, daemon=True)
        self.thread.start()

    def accept(self):
        while not self.closed.is_set():
            try:
                client, _ = self.listener.accept()
            except (TimeoutError, socket.timeout):
                continue
            except OSError:
                return
            threading.Thread(target=self.bridge, args=(client,), daemon=True).start()

    def bridge(self, client):
        upstream = None
        try:
            client.settimeout(10)
            if self.context:
                client = self.context.wrap_socket(client, server_side=True)
            upstream = socket.create_connection(self.upstream, timeout=5)
            with self.lock:
                self.connections.update((client, upstream))
            request_head = bytearray()
            inspected = False
            while not self.closed.is_set():
                if self.blackhole:
                    self.closed.wait(0.05)
                    continue
                ready, _, _ = select.select([client, upstream], [], [], 0.1)
                # SSLSocket may have decrypted bytes buffered without fd readiness.
                if isinstance(client, ssl.SSLSocket) and client.pending() and client not in ready:
                    ready.append(client)
                for source in ready:
                    data = source.recv(16384)
                    if not data:
                        return
                    if source is client and not inspected and self.inspect_http:
                        request_head.extend(data)
                        if len(request_head) > 65536:
                            raise ValueError("HTTP header bound exceeded")
                        if b"\r\n\r\n" not in request_head:
                            continue
                        data = bytes(request_head)
                        method = data.split(b" ", 1)[0].decode("ascii", errors="replace")
                        with self.lock:
                            self.request_counts[method] = self.request_counts.get(method, 0) + 1
                        headers = data.split(b"\r\n\r\n", 1)[0].split(b"\r\n")[1:]
                        upgrade = any(line.lower().split(b":", 1)[0] == b"upgrade" and
                                      line.split(b":", 1)[1].strip().lower() == b"websocket"
                                      for line in headers if b":" in line)
                        request_head.clear()
                        inspected = True
                        if self.reject_websocket and upgrade:
                            client.sendall(b"HTTP/1.1 426 Upgrade Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                            return
                    if source is upstream and self.drop_responses:
                        continue
                    if self.closed.wait(self.delay + (len(data) / self.bytes_per_second if self.bytes_per_second else 0)):
                        return
                    target = upstream if source is client else client
                    target.sendall(data)
        except (OSError, ValueError):
            pass
        finally:
            for stream in (client, upstream):
                if stream:
                    with self.lock:
                        self.connections.discard(stream)
                    stream.close()

    def reset(self):
        with self.lock:
            for stream in list(self.connections):
                try:
                    stream.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
                    stream.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                stream.close()
            self.connections.clear()

    def close(self):
        self.closed.set()
        self.listener.close()
        self.reset()
        self.thread.join(timeout=2)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--cert", required=True)
    parser.add_argument("--key", required=True)
    parser.add_argument("--port", type=int, default=9443)
    parser.add_argument("--upstream", type=int, default=8787)
    parser.add_argument("--reject-websocket", action="store_true")
    options = parser.parse_args()
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(options.cert, options.key)
    proxy = Proxy(("127.0.0.1", options.upstream), port=options.port, context=context,
                  reject_websocket=options.reject_websocket)
    try:
        threading.Event().wait()
    except KeyboardInterrupt:
        proxy.close()
