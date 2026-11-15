"""Verify the actual nginx maps cannot exempt invalid authorization headers."""

# ruff: noqa: S603 - The test runs a local nginx with test-owned configuration.

from http.client import HTTPConnection
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time

import pytest


def test_invalid_tokens_share_the_client_rate_budget(tmp_path):
    nginx = shutil.which("nginx")
    if not nginx:
        pytest.skip("nginx unavailable")
    with socket.socket() as reserve:
        reserve.bind(("127.0.0.1", 0))
        port = reserve.getsockname()[1]
    root = Path(__file__).resolve().parents[1]
    policy = (
        (root / "conf/etc/nginx/conf.d/bot-protection.conf")
        .read_text()
        .replace("/var/cache/nginx/spinorama-compare", str(tmp_path / "cache"))
    )
    (tmp_path / "index.html").write_text("synthetic")
    approved_archive = tmp_path / "docs/data/template Spinorama.tar"
    approved_archive.parent.mkdir(parents=True)
    approved_archive.write_bytes((root / "manual/data/template Spinorama.tar").read_bytes())
    (tmp_path / "policy.conf").write_text(policy)
    (tmp_path / "deny.conf").write_text(
        (root / "conf/etc/nginx/snippets/deny-repository-metadata.conf").read_text()
    )
    config = tmp_path / "nginx.conf"
    config.write_text(f"""daemon off;
pid {tmp_path}/nginx.pid;
error_log {tmp_path}/error.log;
events {{ worker_connections 128; }}
http {{
    access_log off;
    include {tmp_path}/policy.conf;
    server {{
        listen 127.0.0.1:{port};
        root {tmp_path};
        include {tmp_path}/deny.conf;
        location = "/docs/data/template Spinorama.tar" {{ try_files $uri =404; }}
        location / {{ limit_req zone=spin_anonymous_api; limit_req_status 429; }}
    }}
}}
""")
    process = subprocess.Popen(
        [nginx, "-p", str(tmp_path), "-c", str(config)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        for _ in range(100):
            if process.poll() is not None:
                pytest.fail(process.stderr.read().decode())
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                    break
            except OSError:
                time.sleep(0.02)
        statuses = []
        for authorization in [None, None, "Bearer invalid", "Bearer another-invalid"]:
            client = HTTPConnection("127.0.0.1", port, timeout=2)
            client.request(
                "GET",
                "/index.html",
                headers={"Authorization": authorization} if authorization else {},
            )
            response = client.getresponse()
            statuses.append(response.status)
            response.read()
            client.close()
        assert statuses == [200, 429, 429, 429]
        client = HTTPConnection("127.0.0.1", port, timeout=2)
        client.request("GET", "/docs/data/template%20Spinorama.tar")
        response = client.getresponse()
        assert response.status == 200
        assert response.read() == approved_archive.read_bytes()
        client.close()
        for path in [
            "/docs/data/template%20Spinorama.tar.gz",
            "/docs/data/unapproved.tar",
            "/docs/data/measurements.zip",
            "/speakers/Synthetic/CEA2034.json",
            "/speakers/Synthetic/CEA2034.json.br",
            "/speakers/Synthetic/response.csv.gz",
            "/speakers/Synthetic/response.txt",
            "/speakers/Synthetic/traces/CEA2034.webp",
            "/pictures/old.tar",
        ]:
            client = HTTPConnection("127.0.0.1", port, timeout=2)
            client.request("GET", path)
            response = client.getresponse()
            assert response.status == 404, path
            response.read()
            client.close()

    finally:
        process.terminate()
        process.wait(timeout=5)
