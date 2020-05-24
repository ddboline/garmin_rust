#!/bin/bash

PASSWORD=`head -c1000 /dev/urandom | tr -dc [:alpha:][:digit:] | head -c 16; echo ;`
JWT_SECRET=`head -c1000 /dev/urandom | tr -dc [:alpha:][:digit:] | head -c 32; echo ;`
SECRET_KEY=`head -c1000 /dev/urandom | tr -dc [:alpha:][:digit:] | head -c 32; echo ;`
DB=garmin_summary

sudo apt-get install -y postgresql \
    garmin-forerunner-tools \
    fit2tcx \
    libtinyxml2.6.2v5

sudo -u postgres createuser -E -e $USER
sudo -u postgres psql -c "CREATE ROLE $USER PASSWORD '$PASSWORD' NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT LOGIN;"
sudo -u postgres psql -c "ALTER ROLE $USER PASSWORD '$PASSWORD' NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT LOGIN;"
sudo -u postgres createdb $DB

for DIR in ${HOME}/.config/garmin_rust \
           ${HOME}/.garmin_cache/run/gps_tracks \
           ${HOME}/.garmin_cache/run/cache \
           ${HOME}/.garmin_cache/run/summary_cache \
           ${HOME}/.garmin_cache/run/fitbit_cache;
do
    mkdir -p $DIR;
done

cat > ${HOME}/.config/garmin_rust/config.env <<EOL
PGURL=postgresql://$USER:$PASSWORD@localhost:5432/$DB
MAPS_API_KEY=$MAPS_API_KEY
GPS_BUCKET=garmin_scripts_gps_files_ddboline
CACHE_BUCKET=garmin-scripts-cache-ddboline
HTTP_BUCKET=garmin-scripts-http-cache
SUMMARY_BUCKET=garmin-scripts-summary-cache
GPS_DIR=${HOME}/.garmin_cache/run/gps_tracks
CACHE_DIR=${HOME}/.garmin_cache/run/cache
SUMMARY_CACHE=${HOME}/.garmin_cache/run/summary_cache
JWT_SECRET=$JWT_SECRET
SECRET_KEY=$SECRET_KEY
DOMAIN=$DOMAIN
GARMIN_CONNECT_EMAIL=$GARMIN_CONNECT_EMAIL
GARMIN_CONNECT_PASSWORD=$GARMIN_CONNECT_PASSWORD
GOOGLE_SECRET_FILE=$GOOGLE_SECRET_FILE
GOOGLE_TOKEN_PATH=$GOOGLE_TOKEN_PATH
TELEGRAM_BOT_TOKEN=$TELEGRAM_BOT_TOKEN
FITBIT_CLIENTID=$FITBIT_CLIENTID
FITBIT_CLIENTSECRET=$FITBIT_CLIENTSECRET
FITBIT_CACHEDIR=${HOME}/.garmin_cache/run/fitbit_cache
FITBIT_BUCKET=fitbit-cache-ddboline
EOL

psql $DB < ./scripts/garmin_corrections_laps.sql
psql $DB < ./scripts/garmin_summary.sql
psql $DB < ./scripts/strava_id_cache.sql
psql $DB < ./scripts/scale_measurements.sql
