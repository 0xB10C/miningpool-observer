# Developing miningpool-observer

**This documentation page is a stub and should be expanded over time.**

## Rust

Generally, format code with `cargo fmt`.

The `miningpool-observer-daemon` and `miningpool-observer-web` crate share code via the `miningpool_observer_shared` crate.

## Icons and Logos

The icons and logos are created with Inkscape.
Generally, the SVG source files should be saved as Inkscape-SVGs in the [artwork](../artwork) directory.
The Inkscape SVGs used as static assets should be saved as Plain-SVGs or Optimized-SVGs.

The OpenGraph SVG templates located in (artwork/ogimage_templates)[../artwork/ogimage_templates] should be saved as both Inkscape-SVGs and Plain-SVGs.
These should be optimized by running the `optimize-svgs-with-scour.sh` script.
This requires [scour](https://github.com/scour-project/scour) to be installed.

## Testing

On NixOS, the compiler, the PostgreSQL client library, and cargo-mutants are
available through `nix-shell -p gcc postgresql cargo-mutants`.

Run the unit and integration tests with:

```
cargo test --all-features
```

### Database tests

Tests that talk to PostgreSQL are skipped unless `TEST_DATABASE_URL` is set.
The script `contrib/test-db.sh` starts a throwaway cluster for them:

```
eval "$(contrib/test-db.sh start)"
cargo test --all-features
contrib/test-db.sh stop
```

Each database test runs inside a transaction that is rolled back, so the
tests don't interfere with each other.

### Mutation testing

[cargo-mutants](https://mutants.rs) changes small pieces of the code and checks
that at least one test fails. Mutants that no test catches point at missing
tests. The configuration in `.cargo/mutants.toml` skips code that can't run
without a Bitcoin Core node or a database.

```
cargo mutants -p miningpool_observer_shared
cargo mutants -p miningpool-observer-daemon
cargo mutants -p miningpool-observer-web
```

Results are written to `mutants.out/`.
