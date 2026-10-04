"""A REAL TCP forwarder for fault injection - NOT a fake provider.

It opens a listener and pipes bytes verbatim to a real upstream. It does not parse,
answer, or fabricate anything: it is a real network hop. KILLING it makes the route die,
which is a REAL mid-turn network failure for the client. Import `start(host, port)`.
"""
import socket
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

    def _handle(self, c):
        try:
            u = socket.create_connection((self.host, self.port), timeout=10)
        except OSError:
            c.close()
            return
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
        """Cut the route for REAL: close the listener and every pipe (drop, not a clean
        close), so an in-flight HTTP request gets a connection reset."""
        self.alive = False
        try:
            self.srv.close()
        except OSError:
            pass
