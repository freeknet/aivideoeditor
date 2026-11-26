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
