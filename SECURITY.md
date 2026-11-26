# Security Policy

## Reporting a vulnerability

Please do **not** open a public issue for a security vulnerability. Instead,
report it privately through GitHub's private vulnerability reporting on the
[Security tab](https://github.com/freeknet/aivideoeditor/security/advisories/new).

Include what you found, how to reproduce it, and which version or commit you
tested against. Reports are acknowledged within a few days, and fixes land in
the next release.

## Supported versions

Only the latest release is supported. Fixes ship in a new release; there is
no backport window for older versions.

## Where secrets are kept

- **The Anthropic API key**, if you paste one into the assistant's
  settings, is stored **in plain text** in
  `%APPDATA%\aivideoeditor\assistant.toml` (the roaming config folder; on
  Linux `~/.config/aivideoeditor/assistant.toml`). Anyone who can read your
  user profile, or a backup or roaming copy of it, can read the key. It is
  not encrypted and not in the Windows credential store. To keep it out of
  that file, set `ANTHROPIC_API_KEY` in your environment instead (it takes
  precedence) and leave the field empty, or use the Claude Code engine,
  which signs in through its own client and stores no key in ave.
- ave's own log lines (`%LOCALAPPDATA%\aivideoeditor\logs`) do not
  include the key. At `RUST_LOG=debug` or `trace` the HTTP library's own
  logging is enabled too; check a log made at those levels before sharing
  it.

## Scope

What *is* in scope:

- **Untrusted input.** Files, arguments, environment variables and anything
  read from disk or the network are attacker-influenced; a crash, hang or
  unbounded allocation reachable from them is a real finding.
- **Misuse of an `unsafe` block.** The platform surface is hand-wrapped in
  `src/win/`; a handle leak, a missing bounds check, or a struct layout that
  disagrees with what the system writes all belong here.
- **The config file.** A config file that can make the app do something
  other than change its own settings.
- **The release pipeline.** Release archives carry build-provenance and SBOM
  attestations (`gh attestation verify <file> -R freeknet/aivideoeditor`); a way
  to ship an archive that verifies but was not built from the tagged source
  is in scope.
- **The dependency tree.** A vulnerable dependency this project pulls in
  that has not been updated.

Everything else (bug reports, feature requests, and questions) belongs on
the [issue tracker](https://github.com/freeknet/aivideoeditor/issues); see
[`CONTRIBUTING.md`](CONTRIBUTING.md).
