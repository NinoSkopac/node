# myst-consumer-rs

A minimal, self-contained consumer implementation written in Rust. It focuses on doing just enough to:

- import an existing keystore identity;
- optionally talk to a Hermes endpoint without going through Tequilapi; and
- open a TCP proxy toward a provider contact.

## Usage

Build the binary:

```bash
cd consumer-rs
cargo build --release
```

Import a keystore (password can also be provided via `MYST_PASSWORD`):

```bash
./target/release/myst-consumer-rs identities import default "$(cat my_keystore.json)" --password "mysecret"
```

Bring a proxy up on port 10000, resolving contact from the provider identifier when it already contains `host:port` (otherwise pass `--contact`):

```bash
./target/release/myst-consumer-rs connection up 0xprovider --proxy 10000 --contact "provider.host:4050" --password "mysecret" --hermes http://hermes:8889/api/v1
```

The proxy stays alive until `CTRL+C` is received.

## Notes

- No Tequilapi dependencies are used; Hermes is queried directly via HTTP if a URL is supplied.
- The implementation is intentionally minimal and does not aim for feature parity with the Go consumer.
