#!/bin/bash
# Smoke-test the inbound-call allowlist API against a running dev server.
#
# Exercises the cases the handler has to get right: US and 00-prefixed UK
# numbers both normalising to E.164, DELETE by percent-encoded number, and an
# empty number being rejected.
#
# Credentials and host come from the environment so nothing is baked in:
#   FRONA_URL       (default http://localhost:3001)
#   FRONA_IDENTIFIER, FRONA_PASSWORD  (required)
set -euo pipefail

URL="${FRONA_URL:-http://localhost:3001}"
: "${FRONA_IDENTIFIER:?set FRONA_IDENTIFIER to the handle or email of a local account}"
: "${FRONA_PASSWORD:?set FRONA_PASSWORD for that account}"

BODY=$(python3 -c 'import json,os;print(json.dumps({"identifier":os.environ["FRONA_IDENTIFIER"],"password":os.environ["FRONA_PASSWORD"]}))')
RESP=$(curl -s -X POST "$URL/api/auth/login" -H "Content-Type: application/json" -d "$BODY")
TOKEN=$(echo "$RESP" | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
AUTH="Authorization: Bearer $TOKEN"

step() { echo; echo "=== $1 ==="; }

step "GET allowlist (expect empty)"
curl -s "$URL/api/voice/allowlist" -H "$AUTH"

step "POST add US number"
curl -s -X POST "$URL/api/voice/allowlist" -H "$AUTH" \
  -H "Content-Type: application/json" -d '{"phone":"+1 (555) 555-1234"}'

step "POST add UK 00-prefix number"
curl -s -X POST "$URL/api/voice/allowlist" -H "$AUTH" \
  -H "Content-Type: application/json" -d '{"phone":"0044 20 7946 0958"}'

step "GET allowlist (expect both normalised to E.164)"
curl -s "$URL/api/voice/allowlist" -H "$AUTH"

step "DELETE US number"
curl -s -X DELETE "$URL/api/voice/allowlist/%2B15555551234" -H "$AUTH"

step "GET allowlist (expect only the UK number)"
curl -s "$URL/api/voice/allowlist" -H "$AUTH"

step "POST empty number (expect an error)"
curl -s -X POST "$URL/api/voice/allowlist" -H "$AUTH" \
  -H "Content-Type: application/json" -d '{"phone":""}'
echo
