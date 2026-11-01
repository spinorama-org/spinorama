# -*- coding: utf-8 -*-
"""FastAPI bootstrap. Routes live in ``api.routers``; constants in ``api.state``."""

from __future__ import annotations

# Re-exports for back-compat (tests and external callers do
# ``from api.main import app, METADATA, SPINFILES, load_metadata, glob``).
# Removing these would force every consumer to know the new layout.
from glob import glob

from fastapi import Depends, FastAPI
from .auth import require_api_key

from .state import (
    API_VERSION,
    ALIAS_MEASUREMENTS,
    APIFILES,
    HEADPHONE_FILES,
    HEADPHONE_METADATA,
    KNOWN_MEASUREMENTS,
    METADATA,
    SOFTWARE_VERSION,
    SPINFILES,
    lifespan,
    load_headphone_metadata,
    load_metadata,
)
from .routers import headphone, speaker, validation


openapi_tags = [
    {"name": "speaker", "description": "Speaker measurements and metadata"},
    {"name": "headphone", "description": "Headphone measurements and metadata"},
    {"name": "validation", "description": "Data validation endpoints"},
]


app = FastAPI(
    debug=False,
    dependencies=[Depends(require_api_key)],
    swagger_ui_parameters={"persistAuthorization": False},
    title="Spinorama API",
    description="""
## Request an API key

API access requires **manual approval** and a personal API key. There is no
public registration or automatic approval. These docs are available without a key.

Email [pierre@spinorama.org](mailto:pierre@spinorama.org?subject=%5BAUTH%5D%3A%20API%20access%20request)
with the subject **[AUTH]: API access request — your name or project**.

In your email, explain:

- **Who you are:** your name and, if relevant, organization or project.
- **Why you want access:** what you plan to build or research and how you will use the data.
- Whether you intend to publish or redistribute data, and whether your use is commercial.
- Your expected request volume or frequency, if known.

Requests are reviewed individually. If approved, you will receive a key and its
expiry date. A key does not grant permission for commercial use or override the
measurement licenses below.

## Use your key

Send your key in the **Authorization** header on every data request, over HTTPS:

```bash
curl https://api.spinorama.org/v1/brands \
  -H "Authorization: Bearer YOUR_API_KEY"
```

To try requests here, click **Authorize**, paste the key alone (without `Bearer`),
then use **Try it out**. Keep your key private: do not put it in a URL, public
repository, shared notebook, or browser-side application. Do not share it with
other users. Keys are not intentionally persisted by this documentation UI.

Missing, invalid, expired or revoked keys return **401 Unauthorized**. If your
key is lost or exposed, email Pierre to revoke it and request a replacement.
Access may be revoked. An unavailable key database returns **503**, not anonymous access.

Curve responses carry an imperceptible per-key variation (at most 0.05 dB)
used only to identify the source of leaked or commercially republished data.
Downloads are logged per key. Redistribution of API data therefore remains
traceable to the key it was served to.

ErinsAudioCorner measurements are not available through this API.

## Measurement data license

Unless otherwise stated for a specific measurement, measurement data provided
through this API is licensed under
[CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/)
(Creative Commons Attribution-NonCommercial-ShareAlike 4.0 International).

**Commercial use is not permitted under this license.** Commercial use requires
separate permission from the relevant rights holder(s).

You may share and adapt the data for noncommercial purposes. Credit the original
measurement creator and source, retain supplied attribution notices, link to the
license, and indicate any changes. If you share adapted data, use the same license
or a Creative Commons compatible license. Do not impose additional legal or
technological restrictions on the rights granted by the license.

Measurement-specific licenses and source notices take precedence over this
default. This notice concerns measurement data; the software is licensed
separately under GPLv3. See the
[licensing FAQ](https://www.spinorama.org/docs/05_faq.html#measurement-data-license)
for details.
""",
    license_info={
        "name": "Measurement data: CC BY-NC-SA 4.0 (unless otherwise stated)",
        "url": "https://creativecommons.org/licenses/by-nc-sa/4.0/",
    },
    version=SOFTWARE_VERSION,
    lifespan=lifespan,
    openapi_tags=openapi_tags,
)

@app.middleware("http")
async def prevent_private_response_caching(request, call_next):
    response = await call_next(request)
    response.headers["Cache-Control"] = "private, no-store"
    return response

app.include_router(speaker.router)
app.include_router(headphone.router)
app.include_router(validation.router)


__all__ = [
    "API_VERSION",
    "SOFTWARE_VERSION",
    "app",
]
