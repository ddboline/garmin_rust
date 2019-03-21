#!/bin/bash

DB="garmin_summary"

TABLES="
garmin_corrections_laps
garmin_summary"

mkdir -p backup/

for T in $TABLES;
do
    psql $DB -c "DELETE FROM $T";
done
