"""A REAL HTTP proxy that forwards to a REAL provider and can fail MID-RESPONSE.

NOT a fake provider: the upstream is the real provider the test was configured with
(the given provider); this is a real HTTP hop on loopback. It exists so an interrupt case can
inject the ONE condition a plain TCP forwarder cannot: a provider that answers, lets the
first tokens of a streaming turn through, and THEN dies mid-stream (an API error after the
first token). A cut with no bytes forwarded is P7 (network drop); bytes-then-cut is P5
mid-turn.

How it fails: it forwards the response, and as soon as the stream has provably started
answering - the first `content_block_delta` has gone through, or `cut_after_bytes` of body has
been forwarded, whichever comes first - it closes the UPSTREAM socket with SO_LINGER=0 (a real
RST). The client therefore sees a truncated `text/event-stream` - a real mid-flight failure,
not a fabricated error frame.

`sticky=True` (default) cuts EVERY attempt to a cut path, so a harness that RETRIES the model
call cannot succeed on a later try (the turn genuinely fails mid-flight). With `sticky=False`
only the first attempt is cut - useful to observe a harness's retry behaviour.
"""

import os
import socket
import struct
import sys
import threading


class HttpFault:
    def __init__(self, host, port, cut_after_bytes=0, cut_paths=("/v1/messages", "/chat/completions"),
                 listen_port=0, sticky=True):
        # sticky=True: EVERY attempt to a cut path is cut (a provider that keeps failing).
        # sticky=False: only the FIRST attempt is cut (some harnesses then RETRY; a retry
        # that succeeds makes the turn complete - correct, and NOT a mid-turn failure).
        self.host = host
        self.port = port
        # A byte FLOOR for the cut: the stream is cut once the first content delta has been
        # forwarded OR this many response-body bytes have gone through, whichever is first.
        # The first-token trigger normally fires first; this floor guarantees a cut even if a
        # runtime streams without a recognisable `content_block_delta` frame.
        self.cut_after_bytes = cut_after_bytes
        self.cut_paths = tuple(cut_paths)
        self.srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.srv.bind(("127.0.0.1", listen_port))
        self.srv.listen(16)
        self.listen_port = self.srv.getsockname()[1]
        self.alive = True
        self.sticky = sticky
        self.cut_count = 0                  # how many streaming bodies have been cut
        self.cut_done = threading.Event()   # set once a streaming body has been cut
        self._conns = []
        self._lock = threading.Lock()
        threading.Thread(target=self._accept, daemon=True).start()

    @property
    def url(self):
        return f"http://127.0.0.1:{self.listen_port}"

    def _accept(self):
        while self.alive:
            try:
                c, _ = self.srv.accept()
            except OSError:
                return
            threading.Thread(target=self._handle, args=(c,), daemon=True).start()

    @staticmethod
    def _read_head(f):
        """Read request line + headers (up to the blank line). Returns (head, body_reader)."""
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = f.recv(65536)
            if not chunk:
                return head, b""
            head += chunk
        return head, head.split(b"\r\n\r\n", 1)[1]

    def _handle(self, c):
        try:
            u = socket.create_connection((self.host, self.port), timeout=20)
        except OSError:
            c.close()
            return
        with self._lock:
            self._conns.extend([c, u])

        # Read the request head (and any body bytes already buffered) and forward it whole.
        c.settimeout(20)
        head, rest = self._read_head(c)
        if not head:
            c.close(); u.close(); return
        path = head.split(b" ", 2)[1] if head.count(b" ") >= 1 else b""
        if os.environ.get("HTTPFAULT_TRACE"):
            sys.stderr.write("[httpfault] REQ %r cut=%s" % (head.split(b' ')[0] + b' ' + (head.split(b' ')[1] if head.count(b' ')>=1 else b''), self._should_cut(head.split(b' ')[1] if head.count(b' ')>=1 else b'')) + chr(10))
        wants_cut = self._should_cut(path)

        # Forward the request HEAD plus its COMPLETE body. A real model call is a large POST
        # (the system prompt is tens of KB) and may use either Content-Length or chunked
        # transfer-encoding. Forwarding only the first TCP segment leaves the upstream waiting
        # for the rest of the body forever - no response, a stalled turn. So read the body in
        # full before handing the request over.
        body = rest
        hdr = head.lower()
        clen = 0
        for line in head.split(b"\r\n")[1:]:
            if line.lower().startswith(b"content-length:"):
                try:
                    clen = int(line.split(b":", 1)[1].strip())
                except ValueError:
                    clen = 0
        chunked = b"transfer-encoding:" in hdr and b"chunked" in hdr
        if chunked:
            # Read until the chunked body terminator `0\r\n\r\n` (allowing trailing CRLFs).
            while b"\r\n0\r\n\r\n" not in body and not body.endswith(b"0\r\n\r\n"):
                chunk = c.recv(65536)
                if not chunk:
                    break
                body += chunk
        else:
            while len(body) < clen:
                chunk = c.recv(65536)
                if not chunk:
                    break
                body += chunk
        if os.environ.get("HTTPFAULT_TRACE"):
            sys.stderr.write("[httpfault] BODY clen=%d chunked=%s have=%d" % (clen, chunked, len(body)) + chr(10))
        try:
            u.sendall(head + body)
        except OSError:
            c.close(); u.close(); return

        # Forward the response. For a cut path, cut as soon as the stream has provably
        # started answering - the FIRST content delta has been forwarded - OR after
        # `cut_after_bytes` of body, whichever comes first. Then RST so the client's
        # stream is truncated mid-flight (a real mid-turn failure).
        u.settimeout(20)
        forwarded_body = 0
        header_parsed = False
        saw_first_token = False
        try:
            while True:
                try:
                    d = u.recv(65536)
                except OSError:
                    break
                if not d:
                    break
                if not header_parsed and b"\r\n\r\n" in d:
                    header_parsed = True
                    if os.environ.get("HTTPFAULT_TRACE"):
                        sys.stderr.write("[httpfault] RESPHEAD on %r: %r" % (path, d[:80]) + chr(10))
                if wants_cut and header_parsed:
                    forwarded_body += len(d)
                    if b"content_block_delta" in d:
                        saw_first_token = True
                if wants_cut and header_parsed and (saw_first_token or forwarded_body >= self.cut_after_bytes):
                    # Let the bytes already received go out, THEN cut for real.
                    try:
                        c.sendall(d)
                    except OSError:
                        pass
                    self._rst(c, u)
                    self.cut_count += 1
                    self.cut_done.set()
                    self.cut_at = forwarded_body
                    if os.environ.get("HTTPFAULT_TRACE"):
                        sys.stderr.write("[httpfault] CUT path=%r after %d body bytes (first_token=%s)" % (path, forwarded_body, saw_first_token) + chr(10))
                    return
                try:
                    c.sendall(d)
                except OSError:
                    break
        except Exception as e:
            if os.environ.get("HTTPFAULT_TRACE"):
                sys.stderr.write("[httpfault] LOOP-ERR on %r: %r" % (path, e) + chr(10))
        if os.environ.get("HTTPFAULT_TRACE"):
            sys.stderr.write("[httpfault] END path=%r header_parsed=%s body=%d cut_fired=%s" % (path, header_parsed, forwarded_body, self.cut_done.is_set()) + chr(10))
        self._close(c, u)

    def _should_cut(self, path):
        try:
            p = path.decode("latin1")
        except Exception:
            return False
        if not any(p.startswith(x) or x in p for x in self.cut_paths):
            return False
        # Non-sticky: only the first attempt is cut (a retry may then succeed).
        if not self.sticky and self.cut_done.is_set():
            return False
        return True

    @staticmethod
    def _rst(*socks):
        for s in socks:
            try:
                s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
            except OSError:
                pass
            try:
                s.close()
            except OSError:
                pass

    @staticmethod
    def _close(*socks):
        for s in socks:
            try:
                s.close()
            except OSError:
                pass

    def die(self):
        self.alive = False
        try:
            self.srv.close()
        except OSError:
            pass
        with self._lock:
            conns, self._conns = self._conns, []
        self._rst(*conns)
