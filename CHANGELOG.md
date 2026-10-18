# Changelog

What each release changed for you, newest first. Each line is a commit's summary, linked to its full description and diff. Releases before 2.2.2 are described by their release commits.

## 2.6.1 - 2026-10-18

### Fixes

- Resolve beside reqwest 0.13.2 and newer ([`b702db5`](https://github.com/internetdata/sdk-rust/commit/b702db515c7abfd7c66ce1aebb02b69e68365faf))

## 2.6.0 - 2026-10-17

### Features

- Re-pin the spec to 2026.10.16: dynamic client registration ([`694d2fc`](https://github.com/internetdata/sdk-rust/commit/694d2fc9e2dfe3115c9da756a4307b64f76107eb))

## 2.5.4 - 2026-10-16

### Fixes

- Re-pin the spec to 2026.10.15: three descriptions in US spelling ([`de7add2`](https://github.com/internetdata/sdk-rust/commit/de7add212145ff64f40d52762518955a5c89e969))

## 2.5.3 - 2026-10-13

### Fixes

- Re-pin the spec to 2026.10.13: Open databases come with a license ([`6c96f3c`](https://github.com/internetdata/sdk-rust/commit/6c96f3ca7cff6ad934b32b79437896738e7c49f6))

## 2.5.2 - 2026-10-11

### Fixes

- Read an HTTP date only in RFC 9110's three forms, in GMT ([`60178fa`](https://github.com/internetdata/sdk-rust/commit/60178fa9df67aa562fe499cba431591411c79148))
- Trim the API key, and refuse one holding a control character at build ([`1c3de78`](https://github.com/internetdata/sdk-rust/commit/1c3de78969b3556e3b6ebf1930cdba65b4c1f23b))

## 2.5.1 - 2026-10-10

### Fixes

- Re-pin the spec to 2026.10.09: rotating a key needs apikeys.reveal ([`5462c29`](https://github.com/internetdata/sdk-rust/commit/5462c29e908933ca3066d37dcce96b5dc757a407))
- Read a signed Retry-After as no delay, as RFC 9110 has it ([`41b5a44`](https://github.com/internetdata/sdk-rust/commit/41b5a44b9dac3a8943d704f420d501e4482d3bbf))

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
