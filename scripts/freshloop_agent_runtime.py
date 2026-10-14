#!/usr/bin/env python3
"""Run the agent transport with private local credentials and optional LAN routing.

Does not draft, choose a model, or print secrets. A route override changes DNS
only for this process's Nexus host, preserving HTTPS SNI and certificate checks.
"""
import argparse
import ipaddress
import json
import os
from pathlib import Path
import re
import runpy
import socket
import sys
try:
    import tomllib
except ModuleNotFoundError:
    tomllib = None
from urllib.parse import urlparse


def load_toml_nexus(path: Path) -> dict:
    if tomllib is not None:
        with path.open('rb') as stream:
            return tomllib.load(stream)
    text = path.read_text(encoding='utf-8')
    section = None
    nexus = {}
    for raw_line in text.splitlines():
        line = raw_line.split('#', 1)[0].strip()
        if not line:
            continue
        if line.startswith('[') and line.endswith(']'):
            section = line.strip('[]').strip()
            continue
        if section == 'nexus' and '=' in line:
            k, v = line.split('=', 1)
            nexus[k.strip()] = v.strip().strip('"').strip("'")
    return {'nexus': nexus}


def install_route(base, connect_ip):
    host = urlparse(base).hostname
    address = str(ipaddress.ip_address(connect_ip))
    original = socket.getaddrinfo

    def resolve(query, port, *args, **kwargs):
        return original(address if query == host else query, port, *args, **kwargs)

    socket.getaddrinfo = resolve


def main():
    root = Path(__file__).resolve().parents[1]
    config = load_toml_nexus(root / 'config.toml')
    os.environ.setdefault('NEXUS_URL', config['nexus']['api_url'])
    os.environ.setdefault('NEXUS_KEY', config['nexus']['auth_key'])
    os.environ.setdefault('CORTEX_URL', 'http://127.0.0.1:3721')
    # Never redirect an explicitly supplied different environment endpoint.
    if config['nexus'].get('connect_ip') and os.environ['NEXUS_URL'] == config['nexus']['api_url']:
        install_route(os.environ['NEXUS_URL'], config['nexus']['connect_ip'])
    helper = root / 'skills/freshloop-content-agent/scripts/freshloop_agent.py'
    if len(sys.argv) > 1 and sys.argv[1] == 'api-get':
        parser = argparse.ArgumentParser()
        parser.add_argument('--service', choices=['nexus', 'cortex'], default='nexus')
        parser.add_argument('--path', required=True)
        parser.add_argument('--out', required=True)
        parser.add_argument('--user-id', default=None)
        args = parser.parse_args(sys.argv[2:])
        if not args.path.startswith('/api/'):
            parser.error('path must be an /api/ path on the configured service')
        sys.path.insert(0, str(helper.parent))
        import urllib.parse
        from freshloop_agent import Api, save

        class RuntimeApi(Api):
            def __init__(self, base, key, header, user_id=None):
                super().__init__(base, key, header)
                self.user_id = user_id

            def call(self, path, body=None, retry=True, binary=False):
                data = None if body is None else json.dumps(body, ensure_ascii=False).encode()
                headers = {'Content-Type': 'application/json'}
                if self.key:
                    headers[self.header] = self.key
                target_user = self.user_id
                if not target_user:
                    parsed_q = urllib.parse.parse_qs(urllib.parse.urlparse(path).query)
                    if 'user_id' in parsed_q:
                        target_user = parsed_q['user_id'][0]
                if target_user:
                    headers['x-user-id'] = target_user
                for attempt in range(3):
                    try:
                        request = urllib.request.Request(self.base + path, data=data, headers=headers)
                        with urllib.request.urlopen(request, timeout=45, context=self.context) as response:
                            raw = response.read(64 * 1024 * 1024 + 1)
                            if len(raw) > 64 * 1024 * 1024:
                                raise RuntimeError("API response exceeds 64 MiB limit")
                            return raw if binary else (json.loads(raw) if raw else {})
                    except urllib.error.HTTPError as exc:
                        message = exc.read(2048).decode(errors='replace').replace(self.key, '[redacted]') if self.key else exc.read(2048).decode(errors='replace')
                        if exc.code < 500 or not retry or attempt == 2:
                            from freshloop_agent import ApiError
                            raise ApiError(exc.code, message) from None
                    except (urllib.error.URLError, TimeoutError):
                        if not retry or attempt == 2:
                            raise RuntimeError('Request outcome unknown; preserve checkpoint and reconcile before further writes') from None
                    import time
                    time.sleep(2 ** attempt)

        if args.service == 'nexus':
            api = RuntimeApi(os.environ['NEXUS_URL'], os.environ['NEXUS_KEY'], 'X-NEXUS-KEY', user_id=args.user_id)
        else:
            api = RuntimeApi(os.environ['CORTEX_URL'], os.environ.get('CORTEX_API_KEY'), 'X-CORTEX-KEY', user_id=args.user_id)
        save(args.out, api.call(args.path))
        print(json.dumps({'saved': args.out}))
    else:
        sys.path.insert(0, str(helper.parent))
        sys.argv[0] = str(helper)
        runpy.run_path(str(helper), run_name='__main__')


if __name__ == '__main__':
    main()
