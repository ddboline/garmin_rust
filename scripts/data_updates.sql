UPDATE garmin_corrections_laps SET summary_id = (
    SELECT id FROM garmin_summary a WHERE a.begin_datetime = start_time
);

UPDATE fitbit_activities SET summary_id = (
    SELECT id
    FROM garmin_summary a
    WHERE to_char(a.begin_datetime, 'YYYY-MM-DD HH24:MI')
          = to_char(start_time, 'YYYY-MM-DD HH24:MI')
);

UPDATE strava_activities SET summary_id = (
    SELECT id FROM garmin_summary a WHERE a.begin_datetime = start_date
);

UPDATE garmin_connect_activities SET summary_id = (
    SELECT id FROM garmin_summary a WHERE a.begin_datetime = start_time_gmt
);

INSERT INTO race_results_garmin_summary (race_id, summary_id)
    SELECT a.id, b.id
    FROM race_results a
    JOIN garmin_summary b
        ON a.race_filename = b.filename
    WHERE a.race_filename IS NOT NULL;
