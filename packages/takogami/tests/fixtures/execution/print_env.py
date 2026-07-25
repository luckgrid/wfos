#!/usr/bin/env python3
"""Report sealed environment key names only (never values)."""
import json
import os
import sys

keys = sorted(os.environ.keys())
doc = {
    "keys": keys,
    "has_secret_sentinel": "SECRET_SENTINEL" in os.environ,
    "has_herdr": any(k.startswith("HERDR_") for k in keys),
}
sys.stdout.write(json.dumps(doc, separators=(",", ":")))
sys.stdout.write("\n")
