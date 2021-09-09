#!/bin/sh

if [ -n "$CREATE_CONFIG_FROM_ENVVARS" ]; then
  cat > $CONFIG_FILE <<DELIM

# miningpool.observer web configuration file created in entrypoint-web.sh
# to disable the creation of this configuration file unset CREATE_CONFIG_FROM_ENVVARS

database_url = "$DATABASE_URL"
address = "$ADDRESS"
log_level = "$LOG_LEVEL"
www_dir_path = "/app/www"

[site]
    base_url = "$BASE_URL"
    title = "$SITE_TITLE"
    footer = """
      $SITE_FOOTER
    """
DELIM
fi

exec "$@"
