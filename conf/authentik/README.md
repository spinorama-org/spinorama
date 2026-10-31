> Superseded by the native API-key implementation in `src/api/README.md`. Do not deploy this Authentik draft.

# Account-required API with Authentik

This first step protects the entire API host, including Swagger, OpenAPI,
headphone downloads and speaker data. It does not change the public website or
the data available there. The private workspace is unchanged.

Nginx checks each request with Authentik's embedded outpost. Anonymous clients
are redirected to login. Authentik failure denies access. Nginx replaces the
identity header and adds a server-only secret; the API checks both on every
route. Without configuration the API returns 503, never anonymous data.
Keep Gunicorn on its Unix socket; do not expose it on a public TCP port.

## Install on the API VPS

1. Copy this directory to the API VPS. Copy `.env.example` to `.env`, generate
   independent random values for `PG_PASS` and `AUTHENTIK_SECRET_KEY`, and set
   file permissions to 0600. Configure SMTP for invitations/password resets.
   Run `docker compose up -d`. PostgreSQL has no published port; Authentik listens
   only on `127.0.0.1:9000`. Back up the database volume and `.env`.
2. Point `auth.spinorama.org` at the VPS and configure its HTTPS virtual host
   using `nginx.conf.example`. Complete Authentik's initial administrator setup
   at `https://auth.spinorama.org/if/flow/initial-setup/`.
3. Create a **Proxy Provider**, mode **Forward auth (single application)**,
   external host `https://api.spinorama.org`. Create its application and assign
   the provider to the embedded outpost. Set the embedded outpost's
   `authentik_host` to `https://auth.spinorama.org`.
4. Create group `spinorama-api-approved`. Bind that group to the application
   and add approved accounts. Applications without bindings allow all users;
   explicitly bind the group. Keep public self-registration disabled; initially
   create accounts in the admin UI. Any later invitation enrollment must require
   an invitation. Do not configure unauthenticated path exceptions.
5. Run `sudo python3 provision-api-secret.py` once. This creates matching secrets
   for Nginx and Gunicorn without printing them. The API launcher loads
   `/home/spin/run/spin-api/.api-auth.env`; deployment does not overwrite it.
6. Deploy `src/api` and the updated `scripts/gunicorn_start.sh`. Install
   `conf/etc/nginx/sites-available/spinorama-api`, run `sudo nginx -t`, restart
   `spinorama-api` through Supervisor, then reload Nginx. The Nginx upstream and
   Gunicorn launcher both use `/home/spin/run/gunicorn.sock`.

Do not deploy the Python change alone before setting up authentication: it
deliberately blocks all traffic until the proxy and secret are configured.

## Verify before announcing availability

- A fresh browser visiting `/docs` must sign in, then be able to call the API
  using the same-origin Authentik session cookie.
- An account outside the approved group must be refused.
- Anonymous `curl -i https://api.spinorama.org/v1/brands` must redirect to login
  without returning data. Forged `X-Authentik-Uid` headers must not grant access.
- Stopping Authentik must block API access; no cached API data should be served.
- Remove any older Nginx aliases/static locations or alternate vhosts exposing
  API assets without this authentication check. Purge any external API cache.

Command-line clients need an authenticated session cookie for this first step.
Dedicated OAuth access-token support can be added separately. A shared proxy
secret is infrastructure configuration, never a user API key.

Reference: https://docs.goauthentik.io/add-secure-apps/providers/proxy/server_nginx/
