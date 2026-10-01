---
id: analyze_image
provider: file
parameters:
  path:
    type: string
    description: "Path to the image file (PNG, JPEG, GIF or WebP). Bare paths (e.g. screenshot.png) resolve to your workspace."
  question:
    type: string
    description: "What you need to know about the image, stated specifically — e.g. \"What error message is shown in the red banner?\", \"Transcribe the table, keeping its rows and columns\", \"Is the Submit button enabled?\". A focused question gets a more useful answer than \"describe this\"."
required:
  - path
  - question
---
Ask a vision-capable model a question about an image file and get its answer back as text. Use this when you can't view images yourself, or when you need one specific detail from an image (a screenshot, a photo of a document, a chart) and don't need the whole picture in your context. Ask one clear question per call; call again for follow-ups. Answers come from what is visible in the image only.
