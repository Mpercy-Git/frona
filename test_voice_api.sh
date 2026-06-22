#!/bin/bash
set -e
RESP=$(curl -s -X POST http://localhost:3001/api/auth/login -H "Content-Type: application/json" -d '{"identifier":"test@test.com","password":"testpass123"}')
TOKEN=$(echo "$RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")
AUTH="Authorization: Bearer $TOKEN"
echo "=== Test 1: GET allowlist (empty) ==="
curl -s http://localhost:3001/api/voice/allowlist -H "$AUTH"
echo ""
echo "=== Test 2: POST add US number ==="
curl -s -X POST http://localhost:3001/api/voice/allowlist -H "$AUTH" -H "Content-Type: application/json" -d '{"phone":"+1 (555) 555-1234"}'
echo ""
echo "=== Test 3: POST add UK 00-prefix number ==="
curl -s -X POST http://localhost:3001/api/voice/allowlist -H "$AUTH" -H "Content-Type: application/json" -d '{"phone":"0044 20 7946 0958"}'
echo ""
echo "=== Test 4: GET allowlist (both normalized) ==="
curl -s http://localhost:3001/api/voice/allowlist -H "$AUTH"
echo ""
echo "=== Test 5: DELETE US number ==="
curl -s -X DELETE "http://localhost:3001/api/voice/allowlist/%2B15555551234" -H "$AUTH"
echo ""
echo "=== Test 6: GET allowlist (only UK left) ==="
curl -s http://localhost:3001/api/voice/allowlist -H "$AUTH"
echo ""
echo "=== Test 7: POST empty number (error) ==="
curl -s -X POST http://localhost:3001/api/voice/allowlist -H "$AUTH" -H "Content-Type: application/json" -d '{"phone":""}'
echo ""
