UPDATE garmin_corrections_laps SET summary_id = (
    SELECT id FROM garmin_summary a WHERE a.begin_datetime = start_time
) WHERE summary_id IS NULL;

UPDATE fitbit_activities SET summary_id = (
    SELECT id
    FROM garmin_summary a
    WHERE to_char(a.begin_datetime, 'YYYY-MM-DD HH24:MI')
          = to_char(start_time, 'YYYY-MM-DD HH24:MI')
) WHERE summary_id IS NULL;

UPDATE strava_activities SET summary_id = (
    SELECT id FROM garmin_summary a WHERE a.begin_datetime = start_date
) WHERE summary_id IS NULL;

UPDATE garmin_connect_activities SET summary_id = (
    SELECT id FROM garmin_summary a WHERE a.begin_datetime = start_time_gmt
) WHERE summary_id IS NULL;

INSERT INTO race_results_garmin_summary (race_id, summary_id)
    SELECT a.id, b.id
    FROM race_results a
    JOIN garmin_summary b
        ON a.race_filename = b.filename
    LEFT JOIN race_results_garmin_summary c
        ON c.summary_id = b.id
    WHERE a.race_filename IS NOT NULL
        AND c.id IS NULL;



DELETE FROM scale_measurements
WHERE id IN (
    SELECT id FROM (
        SELECT *, row_number() OVER (PARTITION BY date(datetime at time zone 'utc'), mass ORDER BY datetime) FROM scale_measurements
    ) WHERE row_number > 1
);
