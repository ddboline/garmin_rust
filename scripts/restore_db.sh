#!/bin/bash

DB="garmin_summary"
BUCKET=garmin-summary-db-backup

TABLES="
garmin_corrections_laps
garmin_summary
strava_id_cache
scale_measurements
"

mkdir -p backup/

for T in $TABLES;
do
    aws s3 cp s3://${BUCKET}/${T}.sql.gz backup/${T}.sql.gz
    gzip -dc backup/${T}.sql.gz | psql $DB -c "COPY $T FROM STDIN"
done

psql $DB -c "select setval('garmin_corrections_laps_id_seq', (select max(index) from garmin_corrections_laps), TRUE)"
psql $DB -c "select setval('scale_measurements_id_seq', (select max(index) from scale_measurements), TRUE)"
