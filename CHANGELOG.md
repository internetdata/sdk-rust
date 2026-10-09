# Changelog

What each release changed for you, newest first. Each line is a commit's summary, linked to its full description and diff. Releases before 2.2.2 are described by their release commits.

## 2.5.0 - 2026-10-09

### Features

- Re-pin the spec to 2026.10.08, adding the Open databases' open flag ([`df0df9f`](https://github.com/internetdata/sdk-rust/commit/df0df9f059d2b78fd102c52eb3f97b9e43b0732a))

## 2.4.3 - 2026-10-08

### Fixes

- Retry an unreadable download link answer, as a server_error ([`3cdaded`](https://github.com/internetdata/sdk-rust/commit/3cdaded39b01597c618c8f489aeb52df964bcca1))
- Read a Retry-After dated in RFC 850 or asctime form ([`cb76bc6`](https://github.com/internetdata/sdk-rust/commit/cb76bc6811a02c357fc526d4e605f4e724dbfc8c))

## 2.4.2 - 2026-10-04

### Fixes

- Re-pin the spec to 2026.10.03: metadata needs no license ([`848b3f4`](https://github.com/internetdata/sdk-rust/commit/848b3f42449c7665d7522895482e25ee8996fca1))

## 2.4.1 - 2026-10-02

### Fixes

- Require tokio 1.44.2 and chrono 0.4.20, past their advisories ([`b327d4e`](https://github.com/internetdata/sdk-rust/commit/b327d4e460df4fe51374551708d3187f658f0a04))

## 2.4.0 - 2026-09-30

### Features

- Add the authorization code sign-in, with PKCE ([`fdf97d6`](https://github.com/internetdata/sdk-rust/commit/fdf97d66844827b33775c4cc5fd489bb652208a2))

## 2.3.0 - 2026-09-27

### Features

- Re-pin the spec to 2026.09.26, adding its evaluation-sample fields ([`78396b5`](https://github.com/internetdata/sdk-rust/commit/78396b5e52adcf924ecbabd6ef21e1a2f07921ff))

## 2.2.2 - 2026-09-26

### Fixes

- Bound every server-set wait, and refuse a zero OAuth timeout ([`45b27f6`](https://github.com/internetdata/sdk-rust/commit/45b27f6059576c4ca07ad1af831981d8bccefd37))
