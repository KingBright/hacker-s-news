import importlib.util
from pathlib import Path
import socket
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('freshloop_runtime', Path(__file__).with_name('freshloop_agent_runtime.py'))
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class RouteTests(unittest.TestCase):
    def test_route_changes_only_configured_host(self):
        with patch.object(socket, 'getaddrinfo', return_value=[]) as resolver:
            runtime.install_route('https://news.example:8443', '192.168.2.200')
            socket.getaddrinfo('news.example', 8443, type=socket.SOCK_STREAM)
            resolver.assert_called_with('192.168.2.200', 8443, type=socket.SOCK_STREAM)
            socket.getaddrinfo('127.0.0.1', 3721)
            resolver.assert_called_with('127.0.0.1', 3721)
            socket.getaddrinfo('source.example', 443)
            resolver.assert_called_with('source.example', 443)

    def test_bad_route_fails_before_installation(self):
        with patch.object(socket, 'getaddrinfo') as resolver:
            with self.assertRaises(ValueError):
                runtime.install_route('https://news.example', 'not-an-address')
            self.assertIs(socket.getaddrinfo, resolver)


if __name__ == '__main__':
    unittest.main()
