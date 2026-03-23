# Disperse MCP Server

Disperse includes an [MCP (Model Context Protocol)](https://modelcontextprotocol.io/) server
that exposes project release information and configuration validation as tools
for AI assistants.

## Starting the server

```sh
disperse mcp
```

This starts the MCP server over stdio.

## Configuration

### Claude Code

For the current project:

```sh
claude mcp add -s project disperse -- disperse mcp
```

For all projects (global):

```sh
claude mcp add -s user disperse -- disperse mcp
```

Or manually add to your `.mcp.json`:

```json
{
  "mcpServers": {
    "disperse": {
      "command": "disperse",
      "args": ["mcp"]
    }
  }
}
```

## Available tools

### `info`

Show information about a project: current version, pending version, and
release status.

**Parameters:**

| Name   | Type   | Required | Description                                              |
|--------|--------|----------|----------------------------------------------------------|
| `path` | string | no       | Path to the project directory (defaults to current directory) |

### `validate`

Validate the disperse configuration for a project. Checks that referenced
files exist and that version update rules are correct.

**Parameters:**

| Name   | Type   | Required | Description                                              |
|--------|--------|----------|----------------------------------------------------------|
| `path` | string | no       | Path to the project directory (defaults to current directory) |
