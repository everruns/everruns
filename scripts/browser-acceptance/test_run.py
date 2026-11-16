"""Fast harness contract checks; no browser or Docker required."""
import contextlib
import http.server
import importlib.util
import pathlib
import socket
import ssl
import threading
import unittest
import urllib.error
import urllib.request

spec = importlib.util.spec_from_file_location('acceptance', pathlib.Path(__file__).with_name('run.py'))
acceptance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(acceptance)

@contextlib.contextmanager
def running(server):
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        yield server
    finally:
        server.shutdown()
        server.server_close()

class HarnessTests(unittest.TestCase):
    def test_tls_verifies_ca_and_hostname_without_global_trust(self):
        with running(http.server.ThreadingHTTPServer(('127.0.0.1', 0), acceptance.Fixture)) as origin:
            with acceptance.tls_browser({'endpoint': f'ws://127.0.0.1:{origin.server_port}/'}) as secured:
                address = secured['endpoint'].replace('wss://', 'https://')
                trusted = ssl.create_default_context(cafile=secured['test_ca'])
                with urllib.request.urlopen(address, context=trusted) as response:
                    self.assertIn(b'Browser acceptance', response.read())
                with self.assertRaises(urllib.error.URLError) as untrusted:
                    urllib.request.urlopen(address, context=ssl.create_default_context())
                self.assertIsInstance(untrusted.exception.reason, ssl.SSLCertVerificationError)
                with self.assertRaises(urllib.error.URLError) as mismatch:
                    urllib.request.urlopen(address.replace('127.0.0.1', 'localhost'), context=trusted)
                self.assertIsInstance(mismatch.exception.reason, ssl.SSLCertVerificationError)

    def test_fixture_and_missing_route(self):
        with running(http.server.ThreadingHTTPServer(('127.0.0.1', 0), acceptance.Fixture)) as server:
            base = f'http://127.0.0.1:{server.server_port}'
            with urllib.request.urlopen(base + '/') as response:
                self.assertIn(b'id="name"', response.read())
            with urllib.request.urlopen(base + '/redirect') as response:
                self.assertEqual(response.url, base + '/next')
            with self.assertRaises(urllib.error.HTTPError) as error:
                urllib.request.urlopen(base + '/missing')
            self.assertEqual(error.exception.code, 404)
            error.exception.close()

    def test_websocket_fixture_echo(self):
        with running(http.server.ThreadingHTTPServer(('127.0.0.1', 0), acceptance.Fixture)) as server:
            with socket.create_connection(server.server_address, timeout=2) as connection:
                connection.sendall(b'GET /socket HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n')
                response = b''
                while b'\r\n\r\n' not in response:
                    response += connection.recv(4096)
                self.assertIn(b'101 Switching Protocols', response)
                mask = b'abcd'
                payload = b'fixture-echo'
                connection.sendall(bytes([0x81, 0x80 + len(payload)]) + mask + bytes(c ^ mask[i % 4] for i, c in enumerate(payload)))
                stream = connection.makefile('rb')
                self.assertEqual(stream.read(2), bytes([0x81, len(payload)]))
                self.assertEqual(stream.read(len(payload)), payload)
                connection.sendall(b'\x88\x80' + mask)
                self.assertEqual(stream.read(2), b'\x88\x00')
                stream.close()

    def test_authenticated_proxy_rejects_missing_and_wrong_token(self):
        with running(http.server.ThreadingHTTPServer(('127.0.0.1', 0), acceptance.Fixture)) as fixture:
            proxy = acceptance.ThreadedTCP(('127.0.0.1', 0), acceptance.Forward)
            proxy.upstream = fixture.server_address
            proxy.authorization = 'Bearer synthetic-test-only'
            with running(proxy):
                base = f'http://127.0.0.1:{proxy.server_address[1]}/'
                for headers in [{}, {'Authorization': 'Bearer wrong'}]:
                    with self.assertRaises(urllib.error.HTTPError) as error:
                        urllib.request.urlopen(urllib.request.Request(base, headers=headers))
                    self.assertEqual(error.exception.code, 401)
                    error.exception.close()
                request = urllib.request.Request(base, headers={'Authorization': proxy.authorization})
                with urllib.request.urlopen(request) as response:
                    self.assertIn(b'Browser acceptance fixture', response.read())

if __name__ == '__main__':
    unittest.main()
