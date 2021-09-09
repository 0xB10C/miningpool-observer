#!/bin/sh
if [ -n "$CREATE_CONFIG_FROM_ENVVARS" ]; then
  cat > $CONFIG_FILE <<DELIM

# miningpool.observer deamon configuration file created in entrypoint-daemon.sh
# to disable the creation of this configuration file unset CREATE_CONFIG_FROM_ENVVARS

database_url = "$DATABASE_URL"
log_level = "$LOG_LEVEL"
rpc_host = "$RPC_HOST"
rpc_port = $RPC_PORT
rpc_user = "$RPC_USER"
rpc_password = "$RPC_PASSWORD"

[prometheus]
    enable = $PROMETHEUS_ENABLE
    address = "$PROMETHEUS_ADDRESS"
DELIM
fi

exec "$@"
