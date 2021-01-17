CREATE TABLE race_results_garmin_summary (
    id SERIAL PRIMARY KEY,
    race_id INTEGER,
    summary_id INTEGER
);

ALTER TABLE race_results_garmin_summary ADD FOREIGN KEY (race_id) REFERENCES race_results (id);
ALTER TABLE race_results_garmin_summary ADD FOREIGN KEY (summary_id) REFERENCES garmin_summary (id);
