## MCP Servers

You have access to MCP (Model Context Protocol) servers via the `mcpctl` CLI tool. Each server provides a set of tools you can call.

### Discovery

```bash
mcpctl list                          # list available servers
mcpctl <server> --help               # list tools on a server
mcpctl <server> <tool> --help        # show tool parameters
```

### Calling tools

```bash
mcpctl <server> <tool> --param1 value1 --param2 value2
```

Parameters are typed as CLI flags. Use `--help` on any tool to see the exact flags and types.

### Available servers

See `<mcpservers>` section below for your available servers. Run `mcpctl <server> --help` to discover tools.
