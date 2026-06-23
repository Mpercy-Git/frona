#!/bin/bash
TOKEN=*** -s -X POST http://localhost:3001/api/auth/login -H "Content-Type: application/json" -d '{"identifier":"test@test.com","password": "***}' | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")
curl -s http://localhost:3001/api/config -H "Authorization: Bearer *** | python3 -c "
import sys, json
c = json.load(sys.stdin)
v = c.get('voice', {})
print('inbound_enabled:', v.get('inbound_enabled'))
print('inbound_user_id:', v.get('inbound_user_id'))
print('callback_base_url:', v.get('callback_base_url'))
print('inbound_allowlist:', v.get('inbound_allowlist'))
print('provider:', v.get('provider'))
"
