#!/bin/bash

DB="garmin_summary"
BUCKET=garmin-summary-db-backup

TABLES="
garmin_corrections_laps
garmin_summary
strava_id_cache
scale_measurements
fitbit_heartrate
"

mkdir -p backup/

for T in $TABLES;
do
    aws s3 cp s3://${BUCKET}/${T}.sql.gz backup/${T}.sql.gz
    gzip -dc backup/${T}.sql.gz | psql $DB -c "COPY $T FROM STDIN"
done

psql $DB -c "select setval('garmin_corrections_laps_id_seq', (select max(id) from garmin_corrections_laps), TRUE)"
psql $DB -c "select setval('scale_measurements_id_seq', (select max(id) from scale_measurements), TRUE)"
psql $DB -c "select setval('fitbit_heartrate_id_seq', (select max(id) from fitbit_heartrate), TRUE)"
