# CLAUDE.md

The canonical agent guide is [AGENTS.md](AGENTS.md) — read it first. It has the
layout, the conventions, and the portal's gotchas.

Claude-specific notes:

- **`make verify` is the gate.** Run it before calling a change done; it's
  exactly what CI runs (`fmt-check` + `clippy -D warnings` + tests + smoke).
- **Tests are offline.** No test may touch the network or the keychain. If you
  need new portal data, capture it, scrub it per
  `tests/fixtures/README.md`, and add a fixture.
- **This CLI is read-only, and that's enforced in three places** that must stay
  in agreement: `src/writes.rs` (the catalog), the `api` POST guard, and the
  claims in `README.md` / `AGENTS.md`. Changing one means changing all of them.
- **Don't call a write endpoint to "check the shape."** The catalog in
  `src/writes.rs` was built by reading the portal's JavaScript, deliberately
  without invoking anything. These endpoints move real money out of a real bank
  account.
- When a table column looks empty or a balance looks like `0`, read the "zero
  balance" gotcha in AGENTS.md before changing a parser — that behavior is
  intentional.
