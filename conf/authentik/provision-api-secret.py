#!/usr/bin/env python3
"""Run once as root on the API VPS. Never prints or overwrites secrets."""

import os
from pathlib import Path
import pwd
import secrets


def main():
    account = pwd.getpwnam("spin")
    env = Path('/home/spin/run/spin-api/.api-auth.env')
    nginx = Path('/etc/nginx/snippets/spinorama-api-auth-secret.conf')
    if env.exists() or nginx.exists():
        raise SystemExit('Secret files already exist; refusing to replace them.')
    secret = secrets.token_hex(32)
    for path, content in [
        (env, f'SPINORAMA_AUTH_PROXY_SECRET={secret}\n'),
        (nginx, f'proxy_set_header X-Spinorama-Auth-Secret "{secret}";\n'),
    ]:
        path.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'w') as stream:
            stream.write(content)
    os.chown(env, account.pw_uid, account.pw_gid)
    print('Created API and Nginx secret files with mode 0600.')


if __name__ == '__main__':
    main()
