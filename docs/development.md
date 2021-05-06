# Developing miningpool-observer

**This documentation page is a stub and should be expanded over time.**

## Rust

Generally, format code with `cargo fmt`.

The `miningpool-observer-daemon` and `miningpool-observer-web` crate share code via the `miningpool_observer_shared` crate.

## Updating the Sanctioned Address List

The [JSON list of sanctioned addresses](../sanctioned_addresses_XBT.json) is created with this python script [ofac-sanctioned-digital-currency-addresses](https://github.com/0xB10C/ofac-sanctioned-digital-currency-addresses).
This list of sanctioned addresses is automatically updated (checked each night) https://github.com/0xB10C/ofac-sanctioned-digital-currency-addresses/blob/lists/sanctioned_addresses_XBT.json

The JSON list in this repository should be updated as soon as new addresses are added.

The JSON file is used for Rust code generation during compilation. See [daemon/build.rs](../daemon/build.rs) and [web/build.rs](../web/build.rs).

## Icons and Logos

The icons and logos are created with Inkscape.
Generally, the SVG source files should be saved as Inkscape-SVGs in the [artwork](../artwork) directory.
The Inkscape SVGs used as static assets should be saved as Plain-SVGs or Optimized-SVGs.

The OpenGraph SVG templates located in (artwork/ogimage_templates)[../artwork/ogimage_templates] should be saved as both Inkscape-SVGs and Plain-SVGs.
These should be optimized by running the `optimize-svgs-with-scour.sh` script.
This requires [scour](https://github.com/scour-project/scour) to be installed.