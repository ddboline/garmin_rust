#!/bin/bash

DB="garmin_summary"

TABLES="
garmin_corrections_laps
garmin_summary"

for T in $TABLES;
do
    psql $DB -c "COPY $T TO STDOUT" | gzip > backup/${T}.sql.gz
done
