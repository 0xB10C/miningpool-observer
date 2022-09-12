# Self-Hosting miningpool-observer

The miningpool-observer project is built with self-hosting in mind.

![Infrastructure Overview][infra-overview]

[infra-overview]: self-hosting-overview.png

The `miningpool-observer-daemon` connects to a Bitcoin Core instance via the RPC interface and periodically requests block templates.
Once a new block is found, it processes the block and writes information about the templates, blocks, and transactions into the PostgreSQL database.
The database is accessed by the `miningpool-observer-web` web-server, which generates the final HTML pages from the data and site templates.
If the self-hosted miningpool-observer instance should be publically reachable (as opposed to reachable via a local or private network) then it is recommended to proxy connections to the `miningpool-observer-web` web-server e.g. TLS termination and caching of static assets.


This setup can separate the `miningpool-observer-web` web-server host from the host containing the `miningpool-observer-daemon` and the Bitcoin Core node.
The `miningpool-observer-daemon` and `miningpool-observer-web` processes don't communicate directly.
All data is shared via the database.
While the `miningpool-observer-daemon` reads from and writes to the database, the `miningpool-observer-web` process only reads from the database.
As the `miningpool-observer-web` web-server makes multiple SQL queries to the database for a single web request, a low-latency connection between the web-server and the database is recommended.

## Running miningpool-observer

The miningpool-observer project is written in Rust.
An advanced user might want to build the release versions of `miningpool-observer-daemon` and `miningpool-observer-web` himself and create, for example, systemd units for both, but automatically build docker images are provided as well.
A NixOS package and module is available as well (contact @0xB10C for more information).

### Requirements

- Bitcoin Core v22.0 or newer
- A PostgreSQL database version 10 or newer (and disk space for around 150 MB of data per month)

### Database

The `miningpool-observer-daemon` automatically runs database migrations on startup.
No manual table creation required.

### Rust

For installation without Docker, [install Rust](https://www.rust-lang.org/tools/install) first.

You also need to `apt-get -y install libpq-dev`.

### Daemon

The `miningpool-observer-daemon` periodically requests block templates from Bitcoin Core and processes new blocks.
Data is written to and read from the database.

Clone the repository, optionally checkout a release tag and run:

```sh
cargo build --release --bin miningpool-observer-daemon
```

To see if it works, run `target/release/miningpool-observer-daemon`

An example systemd unit is provided [here](/contrib/miningpool-observer-daemon.service), which assumes a user `miningobs`.

#### Configuration

By default, `miningpool-observer-daemon` expects a `daemon-config.toml` configuration file to be present in the current working directory.
A custom path to the configuration file can be set via the `CONFIG_FILE` environment variable.
An example configuration file with placeholders and explanation is provided as [`daemon-config.toml.example`](../daemon-config.toml.example).
Generally, information for the PostgreSQL database and the Bitcoin Core RPC connection must be defined.
Additionally, monitoring via a Prometheus metrics server can be enabled.

#### Docker

The `miningpool-observer-daemon` image docker image can be build from the [Dockerfile.daemon](../contrib/docker/Dockerfile.daemon).
The docker image requires the configuration file to be present under `/app/daemon-config.toml`.
To mount the `daemon-config.toml` file when `docker run`'ing the image a [docker bind mount](https://docs.docker.com/storage/bind-mounts/), like for example `--mount type=bind,source="$(pwd)"/daemon-config.toml,target=/app/daemon-config.toml`, is recommended.

---

### Web

The `miningpool-observer-web` web-server and reads data from the database and generates HTML pages with the data and site templates.

Optionally (if it's a different user) clone the repository, checkout a release tag and run:

```sh
cargo build --release --bin miningpool-observer-web
```

To see if it works, run `target/release/miningpool-observer-web`, but first see Configuration below.

An example systemd unit is provided [here](/contrib/miningpool-observer-web.service), which assumes the same user `miningobs` as the daemon.

#### Configuration

By default, `miningpool-observer-web` expects a `web-config.toml` configuration file to be present in the current working directory.
A custom path to the configuration file can be set via the `CONFIG_FILE` environment variable.
An example configuration file with placeholders and explanation is provided as [`web-config.toml.example`](../web-config.toml.example).
Generally, information for the PostgreSQL database connection and the listening address of the web-server must be configured.
The `www_dir_path` configuration option specifies the path to the [`www`](../www) directory.
This directory contains HTML site templates and static assets.

Additionally, custom site information like the `base_url` (required),  `title`, and `footer` should be set.
The `footer` configuration option allows users to specify custom HTML, which is rendered in the bottom right corner of each page.
This allows for custom branding and, for example, information about donation pages to keep your self-hosted infrastructure running.
[Bootstrap 5 CSS](https://getbootstrap.com/docs/5.0/getting-started/introduction/) classes can be used.

Example footer:
``` html
<div class="my-2">
    <div>
        <span class="text-muted">This site is hosted by</span>
        <br>
        <!-- uncomment this -->
        <!-- span>YOUR NAME / PSEUDONYM</span-->
        <!-- remove this -->
        <span class="badge bg-danger">FIXME: PLACEHOLDER in web-config.toml</span>
    </div>
</div>
```

Renders as:

![Render of the provided example footer HTML][custom-footer]

[custom-footer]: screenshot-placeholder-custom-footer.png

#### Docker

The `miningpool-observer-daemon` image docker image can be build from the [Dockerfile.daemon](../contrib/docker/Dockerfile.daemon).
The docker image requires the configuration file to be present under `/app/daemon-config.toml`.
To mount the `daemon-config.toml` file when `docker run`'ing the image a [docker bind mount](https://docs.docker.com/storage/bind-mounts/), like for example `--mount type=bind,source="$(pwd)"/daemon-config.toml,target=/app/daemon-config.toml`, is recommended.

#### Proxying web requests with Ngnix

The `miningpool-observer-web` web-server does not handle TLS termination and does not cache requests to static pages or assets.
It's recommended to let a reverse proxy such as, for example, Nginx handle this.

Caching for static pages and assets can be enabled with the following example configuration.

```config

http {
    ...

    upstream miningpoolobserver {
        server 127.0.0.1:8000; # should point to the host and port the web-server is listening on
    }
    ...

    # Cache only success status codes; in particular we don't want to cache 404s.
    # See https://serverfault.com/a/690258/128321
    map $status $cache_header {
        200     "public";
        302     "public";
        default "no-cache";
    }

    proxy_cache_path /var/cache/nginx/ levels=1:2 keys_zone=miningpoolcache:10m max_size=1g inactive=6h use_temp_path=off;
    proxy_cache_key "$scheme$request_method$host$request_uri";


    server {
        ...

        location / {
            proxy_pass http://miningpoolobserver;
            ...
        }
        location ~ (/template-and-block/.*|/conflicting/.*) {
            proxy_pass http://miningpoolobserver;
            proxy_cache miningpoolcache;
            proxy_cache_valid 200 302 20m;
            add_header X-Proxy-Cache $upstream_cache_status;
            expires 8h;
            add_header Cache-Control $cache_header always;
            ...
        }
        location /static {
            proxy_pass http://miningpoolobserver;
            proxy_cache miningpoolcache;
            proxy_cache_valid 200 302 60m;
            add_header X-Proxy-Cache $upstream_cache_status;
            expires 30d;
            add_header Cache-Control $cache_header always;
            ...
        }

        ...
    }

}

```
