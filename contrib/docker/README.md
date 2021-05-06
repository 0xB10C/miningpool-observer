# Docker images for miningpool-observer

Build the images with

```
# daemon image
docker build . -f contrib/Dockerfile.daemon

# web image
docker build . -f contrib/Dockerfile.web
```

When using `docker run`, bind mount the configuration files with


```
--mount type=bind,source="$(pwd)"/daemon-config.toml,target=/app/daemon-config.toml
```

and

```
--mount type=bind,source="$(pwd)"/web-config.toml,target=/app/web-config.toml
```