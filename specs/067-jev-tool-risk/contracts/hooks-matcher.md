# Contract: the pre-action hook matcher (067, §6.3.1)

- `integrations/claude-code/hooks.json`, its README and `.claude/settings.json` use a widened `PreToolUse` matcher for the `checkpoint_action` entry that includes MCP tools and excludes `mcp__parallax__*`, verified with no self-trigger.
- A test parses `hooks.json` and checks the matcher.
- The `checkpoint_action` timeout stays 5 s (`hooks.json:18`); the budget contract test (§6.3.5) reads it from this file.
