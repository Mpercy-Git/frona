# Diagnose missing model context

Set `FRONA_LLM_WIRE_TRACE` to a dedicated writable directory and restart Frona.
This opt-in diagnostic works in production/release builds; the older
`FRONA_LLM_TRACE` exchange logger only works in debug builds.

For Docker Compose, add to the server service's environment:

```yaml
FRONA_LLM_WIRE_TRACE: /tmp/frona-wire
```

Recreate the container, reproduce one failed request, then copy the trace files:

```sh
docker cp <container-name>:/tmp/frona-wire ./frona-wire
```

Each `*-request.json` contains the final JSON body sent by the provider transport,
after SDK serialization, prompt caching, and native parameter overrides. The trace
is captured before sending, including attempts that fail. It records no HTTP
headers. Inline media `data:` URL payloads are replaced with their length; message text,
system prompts, tool definitions and routing/compression settings are retained.
Other content, including external URLs and non-data-URL attachments, is retained.
On Unix, files are created with owner-only read/write permissions.

For OpenRouter, inspect `messages` and verify that the last user message contains
the intended instruction and its image block, and that earlier turns are present.
Also inspect `model`, `provider`, `plugins`, `transforms` and `preset` when present.
If text is already missing here, investigate Frona before the transport. If it is
present, investigate the resolved OpenRouter model, server-side transformations
and provider behaviour. A trace cannot show transformations after it leaves Frona.

Traces contain private conversation content. Keep them local, redact any excerpt
before sharing, remove the environment variable and restart after reproduction,
and delete the trace directory when finished. Do not commit traces to a repository.
