"""API key enforcement, public documentation, and administrator lifecycle."""

import hashlib
import sqlite3
import subprocess
import sys

import pytest
from fastapi.testclient import TestClient

from src.api import keys
from src.api.main import app, load_metadata


@pytest.fixture
def access(monkeypatch, tmp_path):
    monkeypatch.setenv('SPINORAMA_API_KEY_DB', str(tmp_path / 'keys.sqlite3'))
    keys.initialize()
    key_id, token = keys.issue('approved@example.org', 'Approved', 'Research')
    app.dependency_overrides[load_metadata] = lambda: {}
    client = TestClient(app)
    try:
        yield client, key_id, token
    finally:
        client.close()
        app.dependency_overrides.clear()


@pytest.mark.parametrize('path', [
    '/v1/brands', '/v1/speakers', '/v1/headphones',
    '/v1/speaker/test/version/test/measurements/On%20Axis',
    '/v1/headphone/test/frequency_response',
])
def test_anonymous_rejected(access, path):
    client, _, _ = access
    response = client.get(path)
    assert response.status_code == 401
    assert response.headers['www-authenticate'] == 'Bearer'
    assert 'no-store' in response.headers['cache-control']


@pytest.mark.parametrize('headers', [
    {'X-Authentik-Uid': 'forged', 'X-Spinorama-Auth-Secret': 'obsolete'},
    {'Authorization': 'Basic arbitrary-token'},
    {'Authorization': 'Bearer arbitrary-token'},
    {'Authorization': 'Bearer'},
])
def test_invalid_credentials(access, headers):
    assert access[0].get('/v1/brands', headers=headers).status_code == 401


def test_valid_key_and_revocation(access):
    client, key_id, token = access
    headers = {'Authorization': 'Bearer ' + token}
    assert client.get('/v1/brands', headers=headers).status_code == 200
    assert keys.revoke(key_id)
    assert client.get('/v1/brands', headers=headers).status_code == 401
    assert not keys.revoke('missing')


def test_expired_key(access, monkeypatch):
    client, _, token = access
    now = keys.time.time()
    monkeypatch.setattr(keys.time, 'time', lambda: now + 366 * 86400)
    assert client.get('/v1/brands', headers={'Authorization': 'Bearer ' + token}).status_code == 401


def test_only_hash_stored(access):
    _, _, token = access
    raw = keys.database_path().read_bytes()
    assert token.encode() not in raw
    assert hashlib.sha256(token.encode()).hexdigest().encode() in raw
    assert keys.database_path().stat().st_mode & 0o777 == 0o600
    with pytest.raises(FileExistsError):
        keys.initialize()


def test_query_string_is_not_a_credential(access):
    client, _, token = access
    assert client.get('/v1/brands', params={'api_key': token}).status_code == 401


@pytest.mark.parametrize('broken', ['missing', 'corrupt'])
def test_database_failure_denies_data_but_keeps_docs(access, broken):
    client, _, token = access
    if broken == 'missing':
        keys.database_path().unlink()
    else:
        keys.database_path().write_bytes(b'not sqlite')
    assert client.get('/v1/brands', headers={'Authorization': 'Bearer ' + token}).status_code == 503
    assert client.get('/docs').status_code == 200
    assert client.get('/openapi.json').status_code == 200


def test_docs_and_security_schema(access):
    client, _, _ = access
    assert client.get('/docs').status_code == 200
    assert client.get('/redoc').status_code == 200
    schema = client.get('/openapi.json').json()
    description = schema['info']['description']
    for text in ['pierre@spinorama.org', '[AUTH]:', 'Who you are', 'Why you want access', 'manual approval']:
        assert text in description
    assert schema['components']['securitySchemes']['APIKey']['scheme'] == 'bearer'
    for path in schema['paths'].values():
        for operation in path.values():
            assert {'APIKey': []} in operation['security']


def test_cli_lifecycle(monkeypatch, tmp_path):
    monkeypatch.setenv('SPINORAMA_API_KEY_DB', str(tmp_path / 'cli.sqlite3'))
    def run(*args):
        return subprocess.run([sys.executable, '-m', 'src.api.keys', *args],
                              capture_output=True, text=True, check=True).stdout
    run('init')
    output = run('issue', '--email', 'a@example.org', '--name', 'A', '--purpose', 'Research', '--days', '30')
    token = output.split('API key (shown only once): ')[1].strip()
    account = keys.lookup(token)
    listing = run('list')
    assert token not in listing and 'digest' not in listing
    assert 'a@example.org' in listing
    run('revoke', account[0])
    assert keys.lookup(token) is None


def test_approvals_are_independent(access):
    _, first, _ = access
    _, other = keys.issue('second@example.org', 'Second', 'Research')
    keys.revoke(first)
    assert keys.lookup(other)[1] == 'second@example.org'
