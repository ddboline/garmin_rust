CREATE INDEX IF NOT EXISTS fitbit_activities_summary_idx ON fitbit_activities (summary_id);
CREATE INDEX IF NOT EXISTS garmin_connect_activities_summary_idx ON garmin_connect_activities (summary_id);
CREATE INDEX IF NOT EXISTS garmin_corrections_laps_summary_idx ON garmin_corrections_laps (summary_id);
CREATE INDEX IF NOT EXISTS race_results_garmin_summary_summary_idx ON race_results_garmin_summary (summary_id);
CREATE INDEX IF NOT EXISTS strava_activities_summary_idx ON strava_activities (summary_id);
CREATE INDEX IF NOT EXISTS race_results_garmin_summary_race_idx ON race_results_garmin_summary (race_id);
CREATE INDEX IF NOT EXISTS garmin_summary_filename_idx ON garmin_summary (filename);
