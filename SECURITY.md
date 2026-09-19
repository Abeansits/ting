# Reporting a vulnerability

Use [GitHub's private vulnerability reporting](https://github.com/Abeansits/ting/security/advisories/new)
when it is available. If the private reporting form is unavailable, open an issue
asking for a private reporting channel without including exploit details,
credentials, or private forum content.

Include the affected Ting version, operating system, minimal reproduction,
expected impact, and any suggested fix. Do not include real secrets.

Security fixes target the latest release and `main`; older releases do not have
a separate maintenance policy. This is a hobby project with best-effort response
times.

## Execution model

Ting runs participant CLI commands as your operating-system user. Custom commands
are shell commands, and provider CLIs enforce their own tool permissions. Ting
does not add a sandbox around them. Use trusted presets and suitable working
directories. The browser server binds to loopback and has no authentication.

Model outputs are untrusted text. They should not be treated as executable code,
authorization, or evidence that a decision is correct. Be deliberate about what
context you send to providers and what you publish in reports.
