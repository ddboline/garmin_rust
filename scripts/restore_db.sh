#!/bin/bash

DB="garmin_summary"

TABLES="
garmin_corrections_laps
garmin_summary"

for T in $TABLES;
do
    gzip -dc backup/${T}.sql.gz | psql $DB -c "COPY $T FROM STDIN"
done
