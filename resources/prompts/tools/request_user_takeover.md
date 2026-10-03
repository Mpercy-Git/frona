---
id: request_user_takeover
provider: human_in_the_loop
parameters:
  reason:
    type: string
    description: Why user intervention is needed
required:
  - reason
---
Request the user to take over the browser session (e.g. for CAPTCHA, 2FA, login). The user gets a live view of the same browser you are driving, on the page you left it, and can click and type in it directly; their cookies and logins stay in the session when they hand it back. Creates a notification and returns immediately.
