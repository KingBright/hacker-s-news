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
import runpy
import socket
import sys
import tomllib
from urllib.parse import urlparse


def install_route(base, connect_ip):
    host = urlparse(base).hostname
    address = str(ipaddress.ip_address(connect_ip))
    original = socket.getaddrinfo

    def resolve(query, port, *args, **kwargs):
        return original(address if query == host else query, port, *args, **kwargs)

    socket.getaddrinfo = resolve


def main():
    root = Path(__file__).resolve().parents[1]
    with (root / 'config.toml').open('rb') as stream:
        config = tomllib.load(stream)
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
        args = parser.parse_args(sys.argv[2:])
        if not args.path.startswith('/api/'):
            parser.error('path must be an /api/ path on the configured service')
        sys.path.insert(0, str(helper.parent))
        from freshloop_agent import Api, save
        if args.service == 'nexus':
            api = Api(os.environ['NEXUS_URL'], os.environ['NEXUS_KEY'], 'X-NEXUS-KEY')
        else:
            api = Api(os.environ['CORTEX_URL'], os.environ.get('CORTEX_API_KEY'), 'X-CORTEX-KEY')
        save(args.out, api.call(args.path))
        print(json.dumps({'saved': args.out}))
    else:
        sys.argv[0] = str(helper)
        runpy.run_path(str(helper), run_name='__main__')


if __name__ == '__main__':
    main()
