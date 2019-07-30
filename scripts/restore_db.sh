#!/bin/bash

DB="garmin_summary"
BUCKET=garmin-summary-db-backup

TABLES="
garmin_corrections_laps
garmin_summary
strava_id_cache
"

mkdir -p backup/

for T in $TABLES;
do
    aws s3 cp s3://${BUCKET}/${T}.sql.gz backup/${T}.sql.gz
    gzip -dc backup/${T}.sql.gz | psql $DB -c "COPY $T FROM STDIN"
done
