#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Headphone-only metadata dump (speaker metadata is Rust generate-meta).

Writes dist/json/headphone_metadata.json and headphone_eqdata.json
from datas/headphones.py plus the EQ/score files under
datas/headphone_eq. Kept in Python because the headphone catalogue
still lives in Python modules; it needs no measurement cache, so it
runs in well under a second.
"""

import logging
import sys

from generate_common import get_custom_logger
from generate_meta import add_headphone_eq, add_headphone_scores, dump_headphone_metadata

logger = logging.getLogger("spinorama")


def main() -> int:
    from datas.headphones import headphones_info

    add_headphone_eq(headphones_info)
    add_headphone_scores(headphones_info)
    dump_headphone_metadata(headphones_info)
    logger.info("Wrote headphone metadata for %d headphones", len(headphones_info))
    return 0


if __name__ == "__main__":
    logger = get_custom_logger(level=logging.INFO, duplicate=True)
    sys.exit(main())
