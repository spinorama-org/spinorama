"""Local administration of individually approved API keys (stdlib only)."""

import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import secrets
import sqlite3
import time


def database_path():
    return Path(os.environ.get('SPINORAMA_API_KEY_DB', '/home/spin/run/spin-api/accounts/keys.sqlite3'))


def connect(mode='ro'):
    return sqlite3.connect(database_path().resolve().as_uri() + '?mode=' + mode, uri=True, timeout=5)


def initialize():
    path = database_path()
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    os.close(fd)
    with closing(connect('rw')) as db, db:
        db.execute('''CREATE TABLE api_keys (
            id TEXT PRIMARY KEY, digest TEXT UNIQUE NOT NULL,
            email TEXT NOT NULL, name TEXT NOT NULL, purpose TEXT NOT NULL,
            created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
            revoked_at INTEGER
        )''')


def issue(email, name, purpose, days=365):
    if not email.strip() or not name.strip() or not purpose.strip() or days <= 0:
        raise ValueError('Email, name, purpose and a positive expiry in days are required')
    token = 'spn_' + secrets.token_urlsafe(32)
    key_id = secrets.token_hex(8)
    now = int(time.time())
    with closing(connect('rw')) as db, db:
        db.execute('INSERT INTO api_keys VALUES (?, ?, ?, ?, ?, ?, ?, NULL)',
                   (key_id, hashlib.sha256(token.encode()).hexdigest(), email.strip(),
                    name.strip(), purpose.strip(), now, now + days * 86400))
    return key_id, token


def lookup(token):
    with closing(connect()) as db:
        return db.execute('''SELECT id, email FROM api_keys
            WHERE digest = ? AND revoked_at IS NULL AND expires_at > ?''',
            (hashlib.sha256(token.encode()).hexdigest(), int(time.time()))).fetchone()


def revoke(key_id):
    with closing(connect('rw')) as db, db:
        return db.execute('UPDATE api_keys SET revoked_at = ? WHERE id = ?',
                          (int(time.time()), key_id)).rowcount == 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    sub.add_parser('init', help='Create the private key database once')
    create = sub.add_parser('issue', help='Approve access and display a new key once')
    for field in ['email', 'name', 'purpose']:
        create.add_argument('--' + field, required=True)
    create.add_argument('--days', type=int, default=365)
    sub.add_parser('list', help='List approvals, expiry and revocation without key hashes')
    sub.add_parser('revoke', help='Revoke a key immediately').add_argument('id')
    args = parser.parse_args()
    try:
        if args.command == 'init':
            initialize()
            print('Initialized', database_path())
        elif args.command == 'issue':
            key_id, token = issue(args.email, args.name, args.purpose, args.days)
            print('Key ID:', key_id)
            print('API key (shown only once):', token)
        elif args.command == 'revoke':
            if not revoke(args.id):
                parser.exit(1, 'Unknown key ID\n')
            print('Revoked', args.id)
        else:
            with closing(connect()) as db:
                db.row_factory = sqlite3.Row
                rows = db.execute('''SELECT id, email, name, purpose, created_at,
                    expires_at, revoked_at FROM api_keys ORDER BY created_at''')
                print(json.dumps([dict(row) for row in rows], indent=2))
    except (OSError, sqlite3.Error, ValueError) as exc:
        parser.exit(1, f'{exc}\n')


if __name__ == '__main__':
    main()
