#!/bin/bash

PASSWORD=`head -c1000 /dev/urandom | tr -dc [:alpha:][:digit:] | head -c 16; echo ;`

sudo apt-get install -y postgresql

sudo -u postgres createuser -E -e $USER
sudo -u postgres psql -c "CREATE ROLE $USER PASSWORD '$PASSWORD' NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT LOGIN;"
sudo -u postgres psql -c "ALTER ROLE $USER PASSWORD '$PASSWORD' NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT LOGIN;"
sudo -u postgres createdb garmin_summary

for DIR in ${HOME}/.config/garmin_rust ${HOME}/.garmin_cache/run/gps_tracks \
           ${HOME}/.garmin_cache/run/cache ${HOME}/.garmin_cache/run/summary_cache;
do
    mkdir -p $DIR;
done

cat > ${HOME}/.config/garmin_rust/config.yml <<EOL
PGURL: postgresql://$USER:$PASSWORD@localhost:5432/garmin_summary
MAPS_API_KEY: $MAPS_API_KEY
GPS_BUCKET: garmin_scripts_gps_files_ddboline
CACHE_BUCKET: garmin-scripts-cache-ddboline
HTTP_BUCKET: garmin-scripts-http-cache
SUMMARY_BUCKET: garmin-scripts-summary-cache
GPS_DIR: ${HOME}/.garmin_cache/run/gps_tracks
CACHE_DIR: ${HOME}/.garmin_cache/run/cache
SUMMARY_CACHE: ${HOME}/.garmin_cache/run/summary_cache
EOL
