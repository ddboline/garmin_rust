#!/bin/bash

DB="garmin_summary"

TABLES="
garmin_corrections_laps
garmin_summary"

mkdir -p backup/

for T in $TABLES;
do
    psql $DB -c "COPY $T TO STDOUT" | gzip > backup/${T}.sql.gz
    aws s3 cp backup/${T}.sql.gz s3://garmin-sumary-db-backup/${T}.sql.gz
done
