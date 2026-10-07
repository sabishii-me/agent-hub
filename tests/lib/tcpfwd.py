"""A REAL TCP forwarder for fault injection - NOT a fake provider.

It opens a listener and pipes bytes verbatim to a real upstream. It does not parse,
answer, or fabricate anything: it is a real network hop. KILLING it makes the route die,
which is a REAL mid-turn network failure for the client. Import `start(host, port)`.
"""
import socket
import struct
import threading


class Forwarder:
    def __init__(self, host, port, listen_port=0):
        self.host = host
        self.port = port
        self.srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.srv.bind(("127.0.0.1", listen_port))
        self.srv.listen(16)
        self.listen_port = self.srv.getsockname()[1]
        self.alive = True
        self._lock = threading.Lock()
        self._conns = []          # every live client+upstream socket, so die() can cut them
        self._t = threading.Thread(target=self._accept, daemon=True)
        self._t.start()

    @property
    def url(self):
        return f"http://127.0.0.1:{self.listen_port}"

    @staticmethod
    def _pipe(a, b):
        try:
            while True:
                d = a.recv(65536)
                if not d:
                    break
                b.sendall(d)
        except OSError:
            pass
        finally:
            try:
                b.shutdown(socket.SHUT_WR)
            except OSError:
                pass

    def _track(self, *socks):
        with self._lock:
            self._conns.extend(socks)

    def _handle(self, c):
        try:
            u = socket.create_connection((self.host, self.port), timeout=10)
        except OSError:
            c.close()
            return
        self._track(c, u)
        threading.Thread(target=self._pipe, args=(c, u), daemon=True).start()
        threading.Thread(target=self._pipe, args=(u, c), daemon=True).start()

    def _accept(self):
        while self.alive:
            try:
                c, _ = self.srv.accept()
            except OSError:
                return
            threading.Thread(target=self._handle, args=(c,), daemon=True).start()

    def die(self):
        """Cut the route for REAL: close the listener AND every established connection with
        SO_LINGER=0, so an IN-FLIGHT request gets a connection reset (RST), not a clean close.
        Closing only the listener leaves established pipes forwarding - not a mid-turn cut."""
        self.alive = False
        try:
            self.srv.close()
        except OSError:
            pass
        with self._lock:
            conns, self._conns = self._conns, []
        for s in conns:
            try:
                s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER,
                             struct.pack("ii", 1, 0))   # linger on, 0s -> RST
            except OSError:
                pass
            try:
                s.close()
            except OSError:
                pass
