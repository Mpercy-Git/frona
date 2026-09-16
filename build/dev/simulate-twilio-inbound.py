#!/usr/bin/env python3
"""Simulate a Twilio inbound call webhook POST with a valid signature."""
import hashlib
import hmac
import base64
import urllib.parse
import sys
import requests

# --- Config ---
TWILIO_AUTH_TOKEN = sys.argv[1] if len(sys.argv) > 1 else "YOUR_TWILIO_AUTH_TOKEN"
CALLER_PHONE = sys.argv[2] if len(sys.argv) > 2 else "+447700900123"
CALL_SID = sys.argv[3] if len(sys.argv) > 3 else "CA_simulated_test_call_001"
BASE_URL = sys.argv[4] if len(sys.argv) > 4 else "https://myfrona.morganpercy.com"

# The URL Twilio would POST to
WEBHOOK_URL = f"{BASE_URL}/api/voice/twilio/inbound"

# Form params that Twilio sends
params = {
    "From": CALLER_PHONE,
    "CallSid": CALL_SID,
    "To": "+441234567890",
    "AccountSid": "AC_test",
    "CallStatus": "ringing",
    "ApiVersion": "2010-04-01",
    "Direction": "inbound",
}

# Compute Twilio signature:
# HMAC-SHA1(auth_token, url + sorted_params_concat)
sorted_params = sorted(params.items())
sig_string = WEBHOOK_URL + "".join(f"{k}{v}" for k, v in sorted_params)

signature = hmac.new(
    TWILIO_AUTH_TOKEN.encode(),
    sig_string.encode(),
    hashlib.sha1
).digest()
signature_b64 = base64.b64encode(signature).decode()

print(f"Target URL: {WEBHOOK_URL}")
print(f"Auth token: {TWILIO_AUTH_TOKEN[:4]}...")
print(f"Caller: {CALLER_PHONE}")
print(f"Signature: {signature_b64}")
print(f"Params: {params}")
print()

# Send the request
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
