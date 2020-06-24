#!/bin/bash

DB="garmin_summary"

TABLES="
garmin_corrections_laps
garmin_summary
strava_id_cache
scale_measurements
heartrate_statistics_summary
fitbit_activities
garmin_connect_activities
strava_activities
"

mkdir -p backup/

for T in $TABLES;
do
    psql $DB -c "DELETE FROM $T";
done
