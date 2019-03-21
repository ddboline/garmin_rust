#!/bin/bash

DB="garmin_summary"

TABLES="
garmin_corrections_laps
garmin_summary"

mkdir -p backup/

for T in $TABLES;
do
    aws s3 cp s3://garmin-sumary-db-backup/${T}.sql.gz backup/${T}.sql.gz
    gzip -dc backup/${T}.sql.gz | psql $DB -c "COPY $T FROM STDIN"
done
