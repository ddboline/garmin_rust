#!/bin/bash

if [ -z "$PASSWORD" ]; then
    PASSWORD=`head -c1000 /dev/urandom | tr -dc [:alpha:][:digit:] | head -c 16; echo ;`
fi
DB=garmin_summary

sudo apt-get install -y postgresql \
    garmin-forerunner-tools \
    fit2tcx \
    libtinyxml2.6.2v5

sudo -u postgres createuser -E -e $USER
sudo -u postgres psql -c "CREATE ROLE $USER PASSWORD '$PASSWORD' NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT LOGIN;"
sudo -u postgres psql -c "ALTER ROLE $USER PASSWORD '$PASSWORD' NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT LOGIN;"
sudo -u postgres createdb $DB
sudo -u postgres psql -c "GRANT ALL PRIVILEGES ON DATABASE $DB TO $USER;"
sudo -u postgres psql $DB -c "GRANT ALL ON SCHEMA public TO $USER;"

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
SECRET_PATH=${HOME}/.config/auth_server_rust/secret.bin
JWT_SECRET_PATH=${HOME}/.config/auth_server_rust/jwt_secret.bin
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

cat > ${HOME}/.config/garmin_rust/postgres.toml <<EOL
[garmin_rust]
database_url = 'postgresql://$USER:$PASSWORD@localhost:5432/$DB'
destination = 'file://${HOME}/setup_files/build/garmin_rust/backup'
tables = ['garmin_corrections_laps', 'garmin_summary', 'scale_measurements', 'heartrate_statistics_summary', 'fitbit_activities', 'garmin_connect_activities', 'strava_activities', 'race_results']
sequences = {garmin_corrections_laps_id_seq=['garmin_corrections_laps', 'id'], scale_measurements_id_seq=['scale_measurements', 'id'], race_results_id_seq=['race_results', 'id']}
EOL

garmin-rust-cli run-migrations
