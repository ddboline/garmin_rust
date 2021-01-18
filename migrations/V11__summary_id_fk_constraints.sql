ALTER TABLE garmin_corrections_laps ADD COLUMN summary_id INTEGER;
ALTER TABLE garmin_corrections_laps ADD FOREIGN KEY (summary_id) REFERENCES garmin_summary (id);
ALTER TABLE fitbit_activities ADD COLUMN summary_id INTEGER;
ALTER TABLE fitbit_activities ADD FOREIGN KEY (summary_id) REFERENCES garmin_summary (id);
ALTER TABLE strava_activities ADD COLUMN summary_id INTEGER;
ALTER TABLE strava_activities ADD FOREIGN KEY (summary_id) REFERENCES garmin_summary (id);
ALTER TABLE garmin_connect_activities ADD COLUMN summary_id INTEGER;
ALTER TABLE garmin_connect_activities ADD FOREIGN KEY (summary_id) REFERENCES garmin_summary (id);
