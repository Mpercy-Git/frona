---
id: send_dtmf
provider: voice_call
parameters:
  digits:
    type: string
    description: DTMF digits to send (0-9, *, #). Example: "1234" or "*"
required:
  - digits
---
Send DTMF (keypad) tones during an active voice call. Use this to navigate IVR menus or enter PINs.
