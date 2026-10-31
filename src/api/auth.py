"""Bearer API key authentication shared by every data route."""

import sqlite3

from fastapi import Depends, HTTPException, Request
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer

from .keys import lookup

bearer = HTTPBearer(
    auto_error=False,
    scheme_name='APIKey',
    description='Request a key from pierre@spinorama.org (subject: [AUTH]: API access request). '
                'Paste only your key here, without the Bearer prefix.',
)


def require_api_key(request: Request, credentials: HTTPAuthorizationCredentials | None = Depends(bearer)):
    if credentials is None or len(credentials.credentials) > 256:
        raise unauthorized()
    try:
        account = lookup(credentials.credentials)
    except (sqlite3.Error, OSError):
        raise HTTPException(503, 'API key service unavailable', headers={'Cache-Control': 'no-store'}) from None
    if account is None:
        raise unauthorized()
    request.state.api_key_id, request.state.account_email = account


def unauthorized():
    return HTTPException(
        401,
        'A valid API key is required. See /docs to request access.',
        headers={'WWW-Authenticate': 'Bearer', 'Cache-Control': 'no-store'},
    )
