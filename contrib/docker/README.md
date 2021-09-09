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

Alternativly, enviroment variables can be used to generate the configuration
file in the entrypoint-{daemon,web}.sh files. Set the
`CREATE_CONFIG_FROM_ENVVARS` variable to make the entrypoints generate a
configuration. Consult the entrypoint-{daemon,web}.sh scripts for the
respective enviroment variables to set.
