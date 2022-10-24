CREATE EXTENSION IF NOT EXISTS pgcrypto;

ALTER TABLE garmin_corrections_laps DROP COLUMN id;
ALTER TABLE garmin_corrections_laps ADD COLUMN id UUID PRIMARY KEY NOT NULL DEFAULT gen_random_uuid();

ALTER TABLE scale_measurements DROP COLUMN id;
ALTER TABLE scale_measurements ADD COLUMN id UUID PRIMARY KEY NOT NULL DEFAULT gen_random_uuid();

ALTER TABLE race_results_garmin_summary DROP COLUMN id;
ALTER TABLE race_results_garmin_summary ADD COLUMN id UUID PRIMARY KEY NOT NULL DEFAULT gen_random_uuid();

ALTER TABLE race_results ADD COLUMN id_temp UUID NOT NULL DEFAULT gen_random_uuid();
ALTER TABLE race_results_garmin_summary ADD COLUMN race_id_temp UUID;

UPDATE race_results_garmin_summary
SET race_id_temp = (
    SELECT distinct id_temp
    FROM race_results
    WHERE id = race_results_garmin_summary.race_id
)
WHERE race_id IS NOT NULL;

ALTER TABLE race_results_garmin_summary DROP CONSTRAINT race_results_garmin_summary_race_id_fkey;

ALTER TABLE race_results DROP COLUMN id;
ALTER TABLE race_results ADD COLUMN id UUID;
UPDATE race_results SET id=id_temp;
ALTER TABLE race_results DROP COLUMN id_temp;
ALTER TABLE race_results ALTER COLUMN id SET NOT NULL;
ALTER TABLE race_results ALTER COLUMN id SET DEFAULT gen_random_uuid();
ALTER TABLE race_results ADD PRIMARY KEY (id);

ALTER TABLE race_results_garmin_summary DROP COLUMN race_id;
ALTER TABLE race_results_garmin_summary ADD COLUMN race_id UUID REFERENCES race_results (id);
UPDATE race_results_garmin_summary SET race_id=race_id_temp;
ALTER TABLE race_results_garmin_summary DROP COLUMN race_id_temp;

ALTER TABLE garmin_summary ADD COLUMN id_temp UUID NOT NULL DEFAULT gen_random_uuid();
ALTER TABLE fitbit_activities ADD COLUMN summary_id_temp UUID;
UPDATE fitbit_activities
SET summary_id_temp = (
    SELECT id_temp
    FROM garmin_summary
    WHERE id = fitbit_activities.summary_id
)
WHERE summary_id IS NOT NULL;
ALTER TABLE fitbit_activities DROP CONSTRAINT fitbit_activities_summary_id_fkey;

ALTER TABLE garmin_connect_activities ADD COLUMN summary_id_temp UUID;
UPDATE garmin_connect_activities
SET summary_id_temp = (
    SELECT id_temp
    FROM garmin_summary
    WHERE id = garmin_connect_activities.summary_id
)
WHERE summary_id IS NOT NULL;
ALTER TABLE garmin_connect_activities DROP CONSTRAINT garmin_connect_activities_summary_id_fkey;

ALTER TABLE garmin_corrections_laps ADD COLUMN summary_id_temp UUID;
UPDATE garmin_corrections_laps
SET summary_id_temp = (
    SELECT id_temp
    FROM garmin_summary
    WHERE id = garmin_corrections_laps.summary_id
)
WHERE summary_id IS NOT NULL;
ALTER TABLE garmin_corrections_laps DROP CONSTRAINT garmin_corrections_laps_summary_id_fkey;

ALTER TABLE race_results_garmin_summary ADD COLUMN summary_id_temp UUID;
UPDATE race_results_garmin_summary
SET summary_id_temp = (
    SELECT id_temp
    FROM garmin_summary
    WHERE id = race_results_garmin_summary.summary_id
)
WHERE summary_id IS NOT NULL;
ALTER TABLE race_results_garmin_summary DROP CONSTRAINT race_results_garmin_summary_summary_id_fkey;

ALTER TABLE strava_activities ADD COLUMN summary_id_temp UUID;
UPDATE strava_activities
SET summary_id_temp = (
    SELECT id_temp
    FROM garmin_summary
    WHERE id = strava_activities.summary_id
)
WHERE summary_id IS NOT NULL;
ALTER TABLE strava_activities DROP CONSTRAINT strava_activities_summary_id_fkey;

ALTER TABLE garmin_summary DROP COLUMN id;
ALTER TABLE garmin_summary ADD COLUMN id UUID;
UPDATE garmin_summary SET id=id_temp;
ALTER TABLE garmin_summary DROP COLUMN id_temp;
ALTER TABLE garmin_summary ALTER COLUMN id SET NOT NULL;
ALTER TABLE garmin_summary ALTER COLUMN id SET DEFAULT gen_random_uuid();
ALTER TABLE garmin_summary ADD PRIMARY KEY (id);

ALTER TABLE fitbit_activities DROP COLUMN summary_id;
ALTER TABLE fitbit_activities ADD COLUMN summary_id UUID REFERENCES garmin_summary (id);
UPDATE fitbit_activities SET summary_id=summary_id_temp;
ALTER TABLE fitbit_activities DROP COLUMN summary_id_temp;

ALTER TABLE garmin_connect_activities DROP COLUMN summary_id;
ALTER TABLE garmin_connect_activities ADD COLUMN summary_id UUID REFERENCES garmin_summary (id);
UPDATE garmin_connect_activities SET summary_id=summary_id_temp;
ALTER TABLE garmin_connect_activities DROP COLUMN summary_id_temp;

ALTER TABLE garmin_corrections_laps DROP COLUMN summary_id;
ALTER TABLE garmin_corrections_laps ADD COLUMN summary_id UUID REFERENCES garmin_summary (id);
UPDATE garmin_corrections_laps SET summary_id=summary_id_temp;
ALTER TABLE garmin_corrections_laps DROP COLUMN summary_id_temp;

ALTER TABLE race_results_garmin_summary DROP COLUMN summary_id;
ALTER TABLE race_results_garmin_summary ADD COLUMN summary_id UUID REFERENCES garmin_summary (id);
UPDATE race_results_garmin_summary SET summary_id=summary_id_temp;
ALTER TABLE race_results_garmin_summary DROP COLUMN summary_id_temp;

ALTER TABLE strava_activities DROP COLUMN summary_id;
ALTER TABLE strava_activities ADD COLUMN summary_id UUID REFERENCES garmin_summary (id);
UPDATE strava_activities SET summary_id=summary_id_temp;
ALTER TABLE strava_activities DROP COLUMN summary_id_temp;
