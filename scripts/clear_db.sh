#!/bin/bash

DB="garmin_summary"

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
    psql $DB -c "DELETE FROM $T";
done
