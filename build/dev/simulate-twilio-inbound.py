#!/usr/bin/env python3
"""Simulate a Twilio inbound-call webhook POST, correctly signed.

Twilio signs each webhook with HMAC-SHA1 over the full URL plus its sorted
form parameters, and the inbound handler rejects anything that doesn't match,
so a plain curl can't exercise the route. This builds the signature the same
way Twilio does.

    build/dev/simulate-twilio-inbound.py <auth-token> [caller] [call-sid] [base-url]

`base-url` is the public URL Twilio would reach, i.e. `voice.callback_base_url`
in your config (default: http://localhost:3001).
"""

import base64
import hashlib
import hmac
import os
import sys

import requests

if len(sys.argv) < 2:
    sys.exit(__doc__)

TWILIO_AUTH_TOKEN = sys.argv[1]
CALLER_PHONE = sys.argv[2] if len(sys.argv) > 2 else "+447700900123"
CALL_SID = sys.argv[3] if len(sys.argv) > 3 else "CA_simulated_test_call_001"
BASE_URL = (
    sys.argv[4]
    if len(sys.argv) > 4
    else os.environ.get("FRONA_VOICE_BASE_URL", "http://localhost:3001")
)

WEBHOOK_URL = f"{BASE_URL.rstrip('/')}/api/voice/twilio/inbound"

params = {
    "From": CALLER_PHONE,
    "CallSid": CALL_SID,
    "To": os.environ.get("FRONA_VOICE_TO_NUMBER", "+441234567890"),
    "AccountSid": "AC_test",
    "CallStatus": "ringing",
    "ApiVersion": "2010-04-01",
    "Direction": "inbound",
}

# HMAC-SHA1(auth_token, url + sorted_params_concat), base64-encoded.
sig_string = WEBHOOK_URL + "".join(f"{k}{v}" for k, v in sorted(params.items()))
signature_b64 = base64.b64encode(
    hmac.new(TWILIO_AUTH_TOKEN.encode(), sig_string.encode(), hashlib.sha1).digest()
).decode()

print(f"Target URL: {WEBHOOK_URL}")
print(f"Caller: {CALLER_PHONE}")
print(f"Signature: {signature_b64}")
print(f"Params: {params}")
print()

resp = requests.post(
    WEBHOOK_URL,
    data=params,
    headers={
        "X-Twilio-Signature": signature_b64,
        "Content-Type": "application/x-www-form-urlencoded",
    },
)

print(f"Status: {resp.status_code}")
print(f"Response: {resp.text[:500]}")
