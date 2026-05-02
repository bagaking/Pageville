# Security policy

Pageville v0 is designed for a single local user. The daemon binds to loopback,
does not provide authentication, and must not be exposed through a LAN address,
reverse proxy, port forward, or public tunnel. Published pages share an origin
with the API, so only publish pages from trusted or deliberately controlled
content.

Please do not disclose an exploitable issue in a public issue before maintainers
have had a chance to investigate. Use GitHub's private vulnerability reporting
for this repository, or contact the maintainers through the repository's private
security channel. Include the affected version, platform, reproduction steps, and
whether the issue requires local file-system access or a browser visiting a page.

The security boundary is the loopback-only v0 deployment. Remote hosting,
authentication, page isolation, and multi-user operation are not supported by this
version.
