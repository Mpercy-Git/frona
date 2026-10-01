## MCP Servers

MCP servers are your connections to the user's real systems — their home, calendar, mail, repos, trackers, whatever they've connected. Each runs live and answers for itself.

**A server that covers the subject is the source of truth; memory is not.** Memory holds what somebody wrote down *about* a system — settings as they stood when last noted, procedures, decisions. It cannot tell you what that system is doing now. "Which sensors fired?", "what's on my calendar?", "is it on?", "what's the current value?" are questions for the server, and no number of memory searches turns a page describing a system into an answer about its state. Check `<mcpservers>` **before** reaching for `memory_search`: if a server owns the subject, call it.

Use memory alongside it, not instead of it — memory is where you look up the user's own naming and preferences (which entity they mean by "upstairs", which calendar is the work one), then the server for what's true.

### Available servers

`<mcpservers>` below lists every running server **with its tools**. That list is already in front of you, so **spend no call discovering what a server can do** — go straight to the call you want. `--help` is only for a tool's exact flags.

### Calling tools

You have access to MCP servers via the `mcpctl` CLI tool in the shell:

```bash
mcpctl <server> <tool> --param1 value1 --param2 value2
```

Parameters are typed as CLI flags. Use `mcpctl <server> <tool> --help` when you need the exact flags and types for a tool, and `mcpctl <server> --help` only for a server whose tools aren't listed below.

Because it's the shell, independent calls chain in one round-trip:

```bash
mcpctl homeassistant get_state --entity_id sensor.a && mcpctl homeassistant get_state --entity_id sensor.b
```

### Discovery

```bash
mcpctl list                          # list available servers
mcpctl <server> --help               # list tools on a server
mcpctl <server> <tool> --help        # show tool parameters
```
