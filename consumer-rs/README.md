# Rust Consumer CLI (`myst`)

This crate contains a minimal Rust reimplementation of the `myst` consumer CLI. It is
focused on supporting the daemon, CLI, and connection workflows needed for the
following commands:

```bash
/usr/bin/myst \
  --config-dir=/var/lib/mysterium-node \
  --script-dir=/var/lib/mysterium-node \
  --data-dir=/var/lib/mysterium-node \
  --runtime-dir=/var/run/mysterium-node \
  --local-service-discovery=false \
  --ui.enable=false \
  --proxymode \
  --tequilapi.address=0.0.0.0 \
  --tequilapi.allowed-hostnames=localhost,.default.svc.cluster.local \
  daemon

myst cli --agreed-terms-and-conditions
myst cli identities import <PASSPHRASE> '<KEYFILE JSON>'
myst connection up --proxy 10000 <PROVIDER_ID>
```

## Prerequisites

* Rust toolchain (1.81 or newer is recommended).
* Access to the internet so Cargo can download dependencies.

Install Rust by following the instructions on <https://rustup.rs/> if you do not
already have it available.

## Building

```bash
cargo build --release
```

The compiled binary will be available at `target/release/myst`.

## Running the daemon

Start the embedded Tequilapi-compatible daemon on a specific address/port:

```bash
cargo run --release -- \
  --config-dir=/var/lib/mysterium-node \
  --script-dir=/var/lib/mysterium-node \
  --data-dir=/var/lib/mysterium-node \
  --runtime-dir=/var/run/mysterium-node \
  --local-service-discovery=false \
  --ui.enable=false \
  --proxymode \
  --tequilapi.address=0.0.0.0 \
  --tequilapi.allowed-hostnames=localhost,.default.svc.cluster.local \
  --tequilapi.port=4050 \
  daemon
```

The flags that mirror the Go CLI are accepted for compatibility, but only the
Tequilapi address/port are currently used by the Rust implementation.

## Accepting terms and importing identities

With the daemon running, you can interact with it using the same binary:

```bash
cargo run --release -- cli --agreed-terms-and-conditions
cargo run --release -- cli identities import "<PASSPHRASE>" '<KEYFILE JSON>'
```

These commands will agree to the required terms of service version and import the
specified keystore JSON for the consumer identity.

## Bringing a connection up

```bash
cargo run --release -- connection up \
  --agreed-terms-and-conditions \
  --proxy 10000 \
  --service-type wireguard \
  <PROVIDER_ID>
```

Multiple provider IDs can be provided by separating them with commas.

## Tests

Run the crate's unit and integration tests with:

```bash
cargo test
```

The test suite includes async mocks that mirror the most important Go-side
workflows for identity import, remote configuration fetching, and smart
connection creation.
