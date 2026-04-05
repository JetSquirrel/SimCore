# simu

A Rust library for Discrete Event Simulation (DES), inspired by Python's SimPy but designed to be idiomatic Rust, high-performance, and scalable.

**Goals:**

- Model complex, process-oriented simulations (e.g. hospital operations, logistics, queuing systems)
- Support thousands of concurrent simulation processes with low overhead
- Reproducible results via seeded RNG
- Monte Carlo parallelism across independent simulation runs using OS threads

For full technical details — architecture, API design, resource model, and roadmap — see [SPEC.md](SPEC.md).

## Usage

Add `simu` to your `Cargo.toml`:

```toml
[dependencies]
simu = { path = "." }
```

Enable parallel Monte Carlo support with the optional feature flag:

```toml
[dependencies]
simu = { path = ".", features = ["monte-carlo"] }
```

## Building

```bash
cargo build
```

## Testing

```bash
cargo test                  # run all tests
cargo test <test_name>      # run a single test
```

## Examples

```bash
cargo run --example hospital
```

## License

TBD
