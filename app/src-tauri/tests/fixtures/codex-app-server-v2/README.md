# Codex app-server v2 protocol fixtures

These fixtures lock the app-server v2 protocol subset used by VoiceCoder, including
thread/turn startup, item updates, automatic review, approval requests, user input,
MCP elicitation, and server-request resolution.

Baseline: `codex-cli 0.144.1`, generated with:

```bash
npm run schema:codex-app-server
```

When the Codex CLI version changes, regenerate the schemas, compare the relevant request and notification definitions, and update these fixtures deliberately.
