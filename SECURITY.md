# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub: **Security → Report a vulnerability** on [osseco/cargo-project-tools](https://github.com/osseco/cargo-project-tools/security/advisories/new). Don't open a public issue.

Include the version (`cargo project-tools --version`), what you did, and what happened. You'll get a reply within a week.

## Supported versions

Only the latest release gets fixes. Releases are immutable, so a fix always ships as a new version.

## Verifying a release

Every release asset is signed with cosign (keyless) and attested by GitHub. See [CONTRIBUTING.md](CONTRIBUTING.md#verifying-a-release).
