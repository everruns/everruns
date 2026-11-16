#!/usr/bin/env python3
"""Run native browser acceptance against a fresh local or containerized Chromium.

The command after -- receives EVERRUNS_BROWSER_CDP_ENDPOINT and
EVERRUNS_BROWSER_FIXTURE_URL. No LLM, personal profile, or extension is used.
"""
import argparse
import base64
import hashlib
import struct
import contextlib
import http.server
import json
import os
import pathlib
import select
import secrets
import shutil
import socket
import socketserver
import ssl
import urllib.parse
import subprocess
import sys
import tempfile
import threading
import time
import uuid

PAGE = b'''<!doctype html><title>Browser acceptance</title>
<h1>Browser acceptance fixture</h1>
<label>Name <input id="name"></label><button id="submit">Submit</button>
<p id="result">Not submitted</p><a id="next" href="/next">Next page</a>
<a id="popup" href="/popup" target="_blank">Open popup</a>
<iframe src="/frame" title="Fixture frame"></iframe>
<script>document.querySelector('#submit').onclick = () => {
const n = Number(localStorage.getItem('submissions') || '0') + 1;
localStorage.setItem('submissions', String(n));
document.querySelector('#result').textContent = document.querySelector('#name').value + ':' + n;
};</script>'''

class Fixture(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def do_GET(self):
        if self.path == '/socket':
            return self.websocket()
        if self.path == '/redirect':
            self.send_response(302)
            self.send_header('Location', '/next')
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        pages = {'/': PAGE, '/next': b'<title>Next page</title><h1>Next page</h1>',
                 '/frame': b'<button id="frame-button">Frame button</button>',
                 '/popup': b'<title>Popup</title><h1>Popup fixture</h1>'}
        body = pages.get(self.path, b'Not found')
        self.send_response(200 if self.path in pages else 404)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def websocket(self):
        # Minimal RFC 6455 text echo for testing page-level browser networking.
        key = self.headers.get('Sec-WebSocket-Key')
        if self.headers.get('Upgrade', '').lower() != 'websocket' or not key:
            self.send_error(400)
            return
        accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
        self.send_response(101)
        self.send_header('Upgrade', 'websocket')
        self.send_header('Connection', 'Upgrade')
        self.send_header('Sec-WebSocket-Accept', accept)
        self.end_headers()
        self.connection.settimeout(10)
        try:
            while True:
                header = self.rfile.read(2)
                if len(header) != 2:
                    return
                opcode, size = header[0] & 15, header[1] & 127
                if not header[0] & 128 or not header[1] & 128 or size == 127:
                    return
                if size == 126:
                    length = self.rfile.read(2)
                    if len(length) != 2:
                        return
                    size = struct.unpack('!H', length)[0]
                mask = self.rfile.read(4)
                body = self.rfile.read(size)
                if len(mask) != 4 or len(body) != size:
                    return
                body = bytes(c ^ mask[i % 4] for i, c in enumerate(body))
                if opcode == 8:
                    self.wfile.write(b'\x88\x00')
                    return
                if opcode not in (1, 9):
                    return
                prefix = bytes([0x8A if opcode == 9 else 0x81])
                length = bytes([size]) if size < 126 else b'\x7e' + struct.pack('!H', size)
                self.wfile.write(prefix + length + body)
                self.wfile.flush()
        except (OSError, TimeoutError):
            return

    def log_message(self, *_args):
        pass

class Forward(socketserver.BaseRequestHandler):
    """Loopback test-only byte proxy, including WebSocket upgraded streams."""
    def handle(self):
        prelude = b''
        if self.server.authorization:
            self.request.settimeout(5)
            while b'\r\n\r\n' not in prelude:
                try:
                    chunk = self.request.recv(4096)
                except (TimeoutError, ConnectionError, OSError):
                    return
                if not chunk or len(prelude) + len(chunk) > 65536:
                    return
                prelude += chunk
            headers = prelude.split(b'\r\n\r\n', 1)[0].split(b'\r\n')[1:]
            values = [line.split(b':', 1)[1].strip().decode('latin1') for line in headers
                      if line.lower().startswith(b'authorization:')]
            if values != [self.server.authorization]:
                self.request.sendall(b'HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n')
                return
        with socket.create_connection(self.server.upstream, timeout=5) as upstream:
            if prelude:
                upstream.sendall(prelude)
            sockets = [self.request, upstream]
            for connection in sockets:
                connection.setblocking(False)
            while True:
                readable, _, _ = select.select(sockets, [], [], 1)
                for source in readable:
                    try:
                        chunk = source.recv(65536)
                    except (ConnectionError, OSError):
                        return
                    if not chunk:
                        return
                    target = upstream if source is self.request else self.request
                    target.setblocking(True)
                    try:
                        target.sendall(chunk)
                    except (ConnectionError, OSError):
                        return
                    finally:
                        target.setblocking(False)

class ThreadedTCP(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

class TLSTCP(ThreadedTCP):
    def get_request(self):
        connection, address = super().get_request()
        try:
            connection.settimeout(5)
            return self.tls.wrap_socket(connection, server_side=True), address
        except Exception:
            connection.close()
            raise

@contextlib.contextmanager
def tls_browser(result):
    """Process-local test CA; never change system/browser trust or disable checks."""
    with tempfile.TemporaryDirectory(prefix='everruns-browser-tls-') as temporary:
        root = pathlib.Path(temporary)
        config, cert, key = root / 'openssl.cnf', root / 'cert.pem', root / 'key.pem'
        ca, ca_key, csr = root / 'ca.pem', root / 'ca-key.pem', root / 'request.pem'
        config.write_text('[req]\ndistinguished_name=dn\nreq_extensions=leaf\nprompt=no\n'
                          '[dn]\nCN=Browser acceptance test\n[ca]\n'
                          'basicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n'
                          '[leaf]\nbasicConstraints=critical,CA:FALSE\n'
                          'subjectAltName=IP:127.0.0.1\nextendedKeyUsage=serverAuth\n'
                          'keyUsage=critical,digitalSignature,keyEncipherment\n')
        commands = [
            ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
             '-config', str(config), '-extensions', 'ca', '-keyout', str(ca_key), '-out', str(ca)],
            ['req', '-newkey', 'rsa:2048', '-nodes', '-config', str(config),
             '-keyout', str(key), '-out', str(csr)],
            ['x509', '-req', '-in', str(csr), '-CA', str(ca), '-CAkey', str(ca_key),
             '-CAcreateserial', '-days', '1', '-extfile', str(config), '-extensions', 'leaf', '-out', str(cert)],
        ]
        for command in commands:
            subprocess.run(['openssl'] + command, check=True, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL)
        key.chmod(0o600)
        ca_key.chmod(0o600)
        endpoint = urllib.parse.urlsplit(result['endpoint'])
        server = TLSTCP(('127.0.0.1', 0), Forward)
        server.upstream = (endpoint.hostname, endpoint.port)
        server.authorization = None  # Inner proxy still validates authorization.
        server.tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        server.tls.load_cert_chain(cert, key)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            yield dict(result, endpoint=f'wss://127.0.0.1:{server.server_address[1]}{endpoint.path}',
                       test_ca=str(ca))
        finally:
            server.shutdown()
            server.server_close()

@contextlib.contextmanager
def browser(worker=False, authenticated=False):
    candidates = [os.environ.get('CHROME_BIN'), shutil.which('chromium'),
                  shutil.which('google-chrome'),
                  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome']
    chrome = next((x for x in candidates if x and pathlib.Path(x).is_file()), None)
    if not chrome:
        raise RuntimeError('Chromium not found; set CHROME_BIN to an installed browser')
    with tempfile.TemporaryDirectory(prefix='everruns-browser-acceptance-') as temporary:
        profile = pathlib.Path(temporary) / 'profile'
        profile.mkdir()
        fixture = http.server.ThreadingHTTPServer(('0.0.0.0' if worker else '127.0.0.1', 8080 if worker else 0), Fixture)
        threading.Thread(target=fixture.serve_forever, daemon=True).start()
        command = [chrome, '--headless=new', '--disable-gpu', '--no-first-run',
                   '--no-default-browser-check', '--disable-background-networking',
                   '--remote-debugging-port=0', f'--user-data-dir={profile}', 'about:blank']
        # The unprivileged, capability-dropped disposable container is the isolation
        # boundary. Never disable the browser sandbox for host execution.
        if worker:
            command.insert(1, '--no-sandbox')
        log = open(pathlib.Path(temporary) / 'chrome.log', 'w+')
        process = subprocess.Popen(command, stdout=log, stderr=log)
        proxy = None
        try:
            active_port = profile / 'DevToolsActivePort'
            deadline = time.monotonic() + 30
            while not active_port.exists():
                if process.poll() is not None or time.monotonic() >= deadline:
                    log.seek(0)
                    raise RuntimeError('Chromium failed to start: ' + log.read()[-2000:])
                time.sleep(.1)
            port, path = active_port.read_text().splitlines()[:2]
            authorization = "Bearer " + secrets.token_urlsafe(32) if authenticated else None
            if worker or authenticated:
                proxy = ThreadedTCP(('0.0.0.0' if worker else '127.0.0.1', 9222 if worker else 0), Forward)
                proxy.upstream = ('127.0.0.1', int(port))
                proxy.authorization = authorization
                threading.Thread(target=proxy.serve_forever, daemon=True).start()
                port = str(proxy.server_address[1])
            yield {'endpoint': f'ws://127.0.0.1:{port}{path}',
                   'fixture': f'http://127.0.0.1:{fixture.server_port}', 'path': path,
                   'authorization': authorization}
        finally:
            if proxy:
                proxy.shutdown()
                proxy.server_close()
            fixture.shutdown()
            fixture.server_close()
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            log.close()

@contextlib.contextmanager
def container_browser(authenticated=False):
    root = pathlib.Path(__file__).resolve().parent
    image = 'everruns-browser-acceptance:local'
    subprocess.run(['docker', 'build', '-t', image, str(root)], check=True)
    name = 'everruns-browser-acceptance-' + uuid.uuid4().hex[:12]
    subprocess.run(['docker', 'run', '-d', '--name', name, '--cap-drop=ALL',
                    '--security-opt=no-new-privileges', '--pids-limit=256',
                    '--memory=1g', '--shm-size=256m', '-p', '127.0.0.1::9222',
                    image] + (['--authenticated'] if authenticated else []), check=True, stdout=subprocess.DEVNULL)
    try:
        deadline = time.monotonic() + 45
        result = None
        while time.monotonic() < deadline:
            discovery = subprocess.run(
                ['docker', 'exec', name, 'cat', '/tmp/browser-acceptance-discovery.json'],
                text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            if discovery.returncode == 0:
                try:
                    result = json.loads(discovery.stdout)
                except ValueError:
                    pass
            if result:
                break
            time.sleep(.2)
        if result is None:
            raise RuntimeError('Container browser did not become ready')
        mapped = subprocess.check_output(['docker', 'port', name, '9222/tcp'], text=True).strip()
        result['endpoint'] = 'ws://' + mapped + result['path']
        yield result
    finally:
        subprocess.run(['docker', 'rm', '-f', name], check=False, stdout=subprocess.DEVNULL)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--container', action='store_true')
    parser.add_argument('--authenticated', action='store_true', help='Require a synthetic bearer token on the loopback CDP proxy')
    parser.add_argument('--tls', action='store_true', help='Exercise verified TLS with an ephemeral process-local test CA')
    parser.add_argument('--worker', action='store_true', help=argparse.SUPPRESS)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.worker:
        with browser(worker=True, authenticated=args.authenticated) as result:
            # Discovery contains a synthetic credential. Keep it out of Docker
            # logs and atomically publish a private, ephemeral runtime file.
            path = '/tmp/browser-acceptance-discovery.json'
            fd = os.open(path + '.tmp', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, 'w') as discovery:
                json.dump(result, discovery)
            os.replace(path + '.tmp', path)
            while True:
                time.sleep(60)
    command = args.command
    if command and command[0] == '--':
        command = command[1:]
    if not command:
        parser.error('Supply the native acceptance-test command after --')
    manager = container_browser(args.authenticated) if args.container else browser(authenticated=args.authenticated)
    with contextlib.ExitStack() as stack:
        result = stack.enter_context(manager)
        if args.tls:
            result = stack.enter_context(tls_browser(result))
        environment = os.environ.copy()
        environment['EVERRUNS_BROWSER_CDP_ENDPOINT'] = result['endpoint']
        environment['EVERRUNS_BROWSER_FIXTURE_URL'] = result['fixture']
        environment['EVERRUNS_BROWSER_ACCEPTANCE_REQUIRED'] = '1'
        if result.get('test_ca'):
            environment['SSL_CERT_FILE'] = result['test_ca']
            environment['SSL_CERT_DIR'] = str(pathlib.Path(result['test_ca']).parent / 'empty')
        if result.get('authorization'):
            environment['EVERRUNS_BROWSER_AUTHORIZATION'] = result['authorization']
        else:
            environment.pop('EVERRUNS_BROWSER_AUTHORIZATION', None)
        completed = subprocess.run(command, env=environment, check=False)
        return completed.returncode

if __name__ == '__main__':
    sys.exit(main())
