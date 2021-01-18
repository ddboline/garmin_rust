ALTER TABLE garmin_summary DROP CONSTRAINT garmin_summary_pkey;
ALTER TABLE garmin_summary ADD UNIQUE (filename);
ALTER TABLE garmin_summary ADD COLUMN id SERIAL PRIMARY KEY;
