# Security

## Reporting

Report vulnerabilities through GitHub Security Advisories for
[LatticeAG/viscompile](https://github.com/LatticeAG/viscompile/security/advisories).
Email is not a second channel until one is published.

Do not attach raw transcripts to public issues. Transcripts can contain secrets,
prompts, and tool arguments that should stay private.

## What this tool trusts

VisCompile trusts local files and stdin JSON that the user passed to `lattice`.
It treats that JSON as untrusted data for rendering: case IDs, strings, and
nested values may contain control bytes, HTML, or DOT metacharacters.

## What this tool never does

`lattice` makes no network requests. It does not execute an agent, call a model
endpoint, or send telemetry. There is no HTTP client in the dependency tree.

## Output escaping

Untrusted text must stay escaped in every renderer:

- Terminal and text reports use `escape_untrusted_text` in `src/types.rs`.
- HTML reports use `push_html_escaped` in `src/format.rs`.
- DOT graphs use `push_dot_escaped` in `src/graph.rs`.
