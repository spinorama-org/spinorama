# API access administration

The API uses manually issued bearer keys and a local SQLite database. No Authentik,
Docker, SSH tunnels or public registration are needed. `/docs`, `/redoc` and
`/openapi.json` are public; all registered data endpoints require a valid key.
Requests for access go to **pierre@spinorama.org**, with subject
**[AUTH]: API access request — name or project**, explaining who the requester is
and why they want access. Issuing a key is the administrator's approval action.

## On the API VPS, as `spin`

```bash
cd /home/spin/run/spin-api
# Once only. Refuses to overwrite an existing database.
.venv/bin/python -m api.keys init

# After reviewing an email request:
.venv/bin/python -m api.keys issue \
  --email 'person@example.org' --name 'Person Name' \
  --purpose 'Approved noncommercial research project' --days 365

.venv/bin/python -m api.keys list
.venv/bin/python -m api.keys revoke KEY_ID
```

The issue command displays the key once. Deliver it privately to the approved
requester with its expiry date. Never put it in Git, URLs, logs or screenshots.
Only a SHA-256 hash of a randomly generated 256-bit token is stored. Listing shows
approval details and Unix timestamps but no key or hash. Lost keys cannot be
recovered: revoke the old ID and issue a replacement. Revoke each key belonging
to an account to withdraw its access. Changes take effect on the next request,
without restarting workers; downloads already in progress are not interrupted.

The default database is `/home/spin/run/spin-api/accounts/keys.sqlite3`, outside
any web root. Override using `SPINORAMA_API_KEY_DB` consistently for the service
and CLI. Initialization creates the database with mode 0600 and a new parent
with mode 0700. Run administration as the API service user. Preserve the database
across deployments and back it up privately using SQLite's backup facility.
A missing or unreadable database denies access with 503. Missing, invalid,
revoked and expired credentials return 401; docs remain available.

## Client usage

```bash
curl https://api.spinorama.org/v1/brands \
  -H 'Authorization: Bearer YOUR_API_KEY'
```

Swagger's **Authorize** button accepts the key without the `Bearer` prefix.
The UI does not persist credentials across page reloads. Clients must use HTTPS.
API-key approval does not change measurement licensing or authorize commercial use.
Per-key rate limiting is not implemented in this first version.

## Deployment

Deploy the `api/` package and `gunicorn_start.sh`, initialize the database as
`spin`, then restart Supervisor's `spinorama-api`. Gunicorn remains on its Unix
socket. Use the Nginx API configuration from `conf/etc/nginx/sites-available/`;
check certificate paths against the server before installing it and run `nginx -t`
before reload. All paths must proxy to FastAPI, without public static-file aliases
or shared caching. The existing Authentik draft is unused.

Verify `/docs` returns 200, anonymous `/v1/brands` returns 401, an issued key
returns 200, and the same key returns 401 after revocation. Check speaker and
headphone downloads as well. No user keys need to be issued to enable the gate.

## Security maintenance

`requirements-api.lock` records the full dependency set tested on the VPS.
Install that lock file for production; `requirements-api.txt` records direct
requirements and security minimums. Update and audit the lock together with the
API regression tests before upgrading production. Upgrade pip itself as well.

Nginx virtual hosts include `snippets/deny-repository-metadata.conf` at server
scope to prevent serving `.git`, `.hg` or `.svn`, including inside other locations.
Install the snippet before testing or reloading a new virtual host. Never include
repository metadata in website deployment artifacts.
