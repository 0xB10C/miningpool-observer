# miningpool-observer

<img alt="miningpool-observer logo" align="right" src="www/static/img/template_and_block.svg" height=100 widht=100>

**Transparency for Mining Pool Transaction Selection**

The miningpool-observer project compares block templates produced by a Bitcoin Core node to blocks produced by mining pools to provide insights about:

- Shared, missing, and extra transactions per template and block pair
- Transactions missing from multiple blocks they should have been included in
- Template and block transactions conflicting with each other
- Blocks not including transactions to or from OFAC sanctioned addresses

This project is inspired by [BitMex Research: Bitcoin Miner Transaction Fee Gathering Capability](https://blog.bitmex.com/bitcoin-miner-transaction-fee-gathering-capability/) and motivated by 9f6f1a8e55623aa320f430f9e3c6dc762c147035e713b96d72c20a58cf45fbbf.

## Self-Hosting

The miningpool-observer project is built with self-hosting in mind.
Both private and public instances, like e.g. [miningpool.observer](https://miningpool.observer), are supported.
Requirements are a Bitcoin Core node v22.0 (currently, you'll need a self-compiled `master` build! requires [PR #18772 (rpc: calculate fees in getblock using BlockUndo data)](https://github.com/bitcoin/bitcoin/pull/18772))) and a PostgreSQL database.

See [docs/self-hosting.md](docs/self-hosting.md) for more information.
## Development

This repository is organized as follows:

```
├── artwork                         # Inkscape sources for the icons and images
├── contrib                         # e.g. Dockerfiles
├── daemon                          # Rust crate for the miningpool-observer-daemon
├── daemon-config.toml.example      # Example configuration file for the miningpool-observer-daemon
├── docs                            # Documentation
├── migrations                      # SQL files automatically ran by the miningpool-observer-daemon on startup
├── sanctioned_addresses_XBT.json   # Contains sanctioned Bitcoin addresses
├── shared                          # Rust crate for code shared between the miningpool-observer-daemon and miningpool-observer-web
├── web                             # Rust crate for the miningpool-observer-web (web-server)
├── web-config.toml.example         # Example configuration file for the miningppool-observer-web
└── www                             # Static resources and HTML page templates used by the miningpool-observer-web web-server
```

See [docs/development.md](docs/development.md) for more information.

## License

This work is licensed under the MIT License.

See [LICENSE](LICENSE) for more information.