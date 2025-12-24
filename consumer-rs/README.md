# Myst Consumer (Rust)

Minimal stand‑alone consumer CLI that talks directly to Tequilapi. It supports the bare essentials to import an identity and bring a connection up to a provider, waiting until the connection reaches `Connected`.

## Building

From the repo root:

```bash
cd consumer-rs
cargo build
```

## Commands

### Import an identity

```bash
# Passphrase plus keystore JSON (or path to a file containing the JSON)
./target/debug/myst-consumer \
  --tequilapi-address 127.0.0.1 \
  --tequilapi-port 4050 \
  cli identities import "<passphrase>" "<keystore-json-or-path>"

# Avoid shell brace expansion by piping JSON:
cat key.json | ./target/debug/myst-consumer \
  --tequilapi-address 127.0.0.1 \
  --tequilapi-port 4050 \
  cli identities import "<passphrase>" --stdin
```

### Bring a connection up

```bash
./target/debug/myst-consumer \
  --tequilapi-address 127.0.0.1 \
  --tequilapi-port 4050 \
  connection up \
  --proxy 10000 \
  --service-type wireguard \
  --dns auto \
  --wait-timeout-secs 60 \
  --status-poll-interval-secs 2 \
  "provider_id_here"
```

Notes:
- Multiple providers can be supplied, comma-separated (e.g., `"id1,id2"`).
- The client fetches Hermes from remote config, agrees to terms if needed, and polls `/connection` until status becomes `Connected` or the timeout expires.
- Kill switch is disabled by default in this minimal client. DNS is configurable via `--dns` (auto/provider/system/custom).
