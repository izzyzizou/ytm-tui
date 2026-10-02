# Security policy

ytm-tui stores YouTube session cookies when you run `ytm-tui auth import`. Anyone who gets
those cookies can act as your Google account, so we treat credential leaks as security bugs.

## Reporting a vulnerability

Please **don't open a public issue.** Report privately through GitHub's
[private vulnerability reporting](https://github.com/izzyzizou/ytm-tui/security/advisories/new).
Include steps to reproduce if you can. Expect an acknowledgement within a week.

In scope: credentials reaching logs, files, or other local users; the daemon socket being
reachable by other users; argument or command injection into `mpv` or `yt-dlp`; crashes
caused by untrusted network or IPC input.

## If you leaked your own cookies

If you pasted a `cookie:` header or an auth file somewhere public, sign out of that browser
session (Google Account → Security → Your devices). That invalidates the cookies. Then run
`ytm-tui auth import` again.

## Supported versions

Only the latest release and `main` get security fixes.
