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
Request the user to take over the browser session (e.g. for CAPTCHA, 2FA, login). The debugger URL is automatically generated for the same browser profile the browser tools use (the one tied to the most recently added credential). Creates a notification and returns immediately.
