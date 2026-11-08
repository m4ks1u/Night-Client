# Security policy

## Supported versions

Night Client is pre-release. Only the latest commit on `main` receives security fixes until the
first public beta.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's
[private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)
on this repository. Include steps to reproduce and the impact you expect.

We aim to acknowledge reports within 3 working days and to agree a disclosure timeline with you.

## Scope

In scope:
- Token or credential exposure (logs, IPC, files, crash reports).
- Path traversal or arbitrary file write from metadata, mods or archives.
- Command or argument injection when launching Java.
- Bypassing integrity verification of downloads or updates.
- WebView/IPC privilege escalation (capabilities, CSP).

Out of scope: vulnerabilities in Minecraft itself or in third-party mods. Report those to their authors.

## Design commitments

- Microsoft sign-in uses OAuth 2.0 with PKCE in the system browser. Night Client never sees your password.
- Refresh tokens are stored only encrypted with Windows DPAPI (current user) in `%LOCALAPPDATA%\NightClient\vault`, never in SQLite, logs or plain files. Minecraft access tokens stay in memory.
- User-supplied JVM arguments are parsed without a shell and checked against an allowlist. Agents, classpath changes and command-running flags are refused.
- Every downloaded file is checked against an official hash when one is published.
- Application updates are signed. Signing keys are never stored in this repository.
