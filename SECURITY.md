# Security policy

## Reporting a vulnerability

Please report security issues through GitHub's private vulnerability reporting
on this repository: **Security** tab → **Report a vulnerability**. Do not open
a public issue for anything that could expose credentials or remote access.

## Scope

alaska stores OpenSubsonic credentials in plain text in the user's
`~/.config/alaska/config.toml` (mode 600). Reports about that design are
welcome, but it is documented behavior. Never include real server URLs,
usernames or passwords in issues, pull requests or logs.

## Supported versions

Only the latest release receives fixes.
