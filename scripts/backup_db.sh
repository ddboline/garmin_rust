#!/bin/bash

DB="garmin_summary"
BUCKET=garmin-summary-db-backup

TABLES="
garmin_corrections_laps
garmin_summary
strava_id_cache
scale_measurements
heartrate_statistics_summary
fitbit_activities
garmin_connect_activities
"

mkdir -p backup/

for T in $TABLES;
do
    psql $DB -c "COPY $T TO STDOUT" | gzip > backup/${T}.sql.gz
    aws s3 cp backup/${T}.sql.gz s3://${BUCKET}/${T}.sql.gz
done
