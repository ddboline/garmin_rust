use anyhow::{format_err, Error};
use smallvec::SmallVec;
use stack_string::StackString;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};
use time::{
    macros::{format_description, time},
    Date, PrimitiveDateTime, Time,
};

use garmin_lib::date_time_wrapper::DateTimeWrapper;
use garmin_models::{
    garmin_correction_lap::{apply_lap_corrections, GarminCorrectionLap},
    garmin_file::GarminFile,
    garmin_lap::GarminLap,
    garmin_point::GarminPoint,
};
use garmin_utils::{
    garmin_util::{convert_time_string, METERS_PER_MILE},
    sport_types::{get_sport_type_map, SportTypes},
};

use super::garmin_parse::{GarminParseTrait, ParseOutput};

#[derive(Debug, Default)]
pub struct GarminParseTxt {}

impl GarminParseTxt {
    #[must_use]
    pub fn new() -> Self {
        Self {}
    }
}

#[allow(clippy::similar_names)]
impl GarminParseTrait for GarminParseTxt {
    fn with_file(
        self,
        filename: &Path,
        corr_map: &HashMap<(DateTimeWrapper, i32), GarminCorrectionLap>,
    ) -> Result<GarminFile, Error> {
        let file_name = filename
            .file_name()
            .ok_or_else(|| format_err!("filename {filename:?} has no path"))?
            .to_string_lossy()
            .to_string()
            .into();
        let txt_output = self.parse_file(filename)?;
        let sport: SportTypes = txt_output
            .lap_list
            .first()
            .ok_or_else(|| format_err!("No laps"))?
            .lap_type
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(SportTypes::None);
        let (lap_list, sport) = apply_lap_corrections(&txt_output.lap_list, sport, corr_map);
        let first_lap = lap_list.first().ok_or_else(|| format_err!("No laps"))?;
        let gfile = GarminFile {
            filename: file_name,
            filetype: "txt".into(),
            begin_datetime: first_lap.lap_start,
            sport,
            total_calories: lap_list.iter().map(|lap| lap.lap_calories).sum(),
            total_distance: lap_list.iter().map(|lap| lap.lap_distance).sum(),
            total_duration: lap_list.iter().map(|lap| lap.lap_duration).sum(),
            total_hr_dur: lap_list
                .iter()
                .map(|lap| lap.lap_avg_hr.unwrap_or(0.0) * lap.lap_duration)
                .sum(),
            total_hr_dis: lap_list.iter().map(|lap| lap.lap_duration).sum(),
            laps: lap_list,
            points: txt_output.point_list,
        };
        Ok(gfile)
    }

    fn parse_file(&self, filename: &Path) -> Result<ParseOutput, Error> {
        let lap_list = Self::get_lap_list(filename)?
            .into_iter()
            .enumerate()
            .map(|(i, lap)| {
                let mut new_lap = lap;
                new_lap.lap_index = i.try_into()?;
                new_lap.lap_number = if new_lap.lap_number == -1 {
                    i.try_into()?
                } else {
                    new_lap.lap_number
                };
                Ok(new_lap)
            })
            .collect::<Result<Vec<_>, Error>>()?;

        let mut time_since_begin = 0.0;
        let mut distance_since_begin = 0.0;
        let duration_speed_vec: Vec<_> = lap_list
            .iter()
            .map(|lap| {
                let duration_from_last = lap.lap_duration;
                let distance_from_last = lap.lap_distance;
                time_since_begin += duration_from_last;
                distance_since_begin += distance_from_last;

                let speed_permi = if distance_from_last > 0.0 {
                    (duration_from_last / 60.) / (distance_from_last / METERS_PER_MILE)
                } else {
                    0.0
                };
                let speed_mph = if duration_from_last > 0.0 {
                    (distance_from_last / METERS_PER_MILE) / (duration_from_last / 3600.)
                } else {
                    0.0
                };
                let speed_mps = if duration_from_last > 0.0 {
                    distance_from_last / duration_from_last
                } else {
                    0.0
                };
                let avg_speed_value_permi = if distance_since_begin > 0.0 {
                    (time_since_begin / 60.) / (distance_since_begin / METERS_PER_MILE)
                } else {
                    0.0
                };
                let avg_speed_value_mph = if time_since_begin > 0.0 {
                    (distance_since_begin / METERS_PER_MILE) / (time_since_begin / 3600.)
                } else {
                    0.0
                };

                (
                    duration_from_last,
                    time_since_begin,
                    speed_permi,
                    speed_mph,
                    speed_mps,
                    avg_speed_value_permi,
                    avg_speed_value_mph,
                )
            })
            .collect();

        let point_list: Vec<_> = lap_list
            .iter()
            .zip(duration_speed_vec.iter())
            .map(
                |(
                    lap,
                    (
                        duration_from_last,
                        time_since_begin,
                        speed_permi,
                        speed_mph,
                        speed_mps,
                        avg_speed_value_permi,
                        avg_speed_value_mph,
                    ),
                )| GarminPoint {
                    time: lap.lap_start,
                    latitude: None,
                    longitude: None,
                    altitude: None,
                    distance: Some(lap.lap_distance),
                    heart_rate: None,
                    duration_from_last: *duration_from_last,
                    duration_from_begin: *time_since_begin,
                    speed_mps: *speed_mps,
                    speed_permi: *speed_permi,
                    speed_mph: *speed_mph,
                    avg_speed_value_permi: *avg_speed_value_permi,
                    avg_speed_value_mph: *avg_speed_value_mph,
                },
            )
            .collect();

        Ok(ParseOutput {
            lap_list,
            point_list,
            sport: SportTypes::None,
        })
    }
}

impl GarminParseTxt {
    fn get_lap_list(filename: &Path) -> Result<Vec<GarminLap>, Error> {
        if !filename.exists() {
            return Err(format_err!("file {filename:?} does not exist"));
        }
        let file = File::open(filename)?;
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut lap_list = Vec::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            if let Ok(pl) = Self::parse_line(&line) {
                lap_list.push(pl);
            }
        }
        Ok(lap_list)
    }

    fn parse_line(line: &str) -> Result<GarminLap, Error> {
        let sport_type_map = get_sport_type_map();

        let entry_dict: HashMap<_, _> = line
            .split_whitespace()
            .filter_map(|x| {
                let entries: SmallVec<[&str; 2]> = x.split('=').take(2).collect();
                if let Some(key) = entries.first() {
                    if let Some(val) = entries.get(1) {
                        let key: StackString = (*key).into();
                        let val: StackString = val.trim().into();
                        return Some((key, val));
                    }
                }
                None
            })
            .collect();

        let date = match entry_dict.get("date") {
            Some(val) => Date::parse(val, format_description!("[year][month][day]"))?,
            None => return Err(format_err!("No date value")),
        };

        let time = if let Some(val) = entry_dict.get("time") {
            Time::parse(val, format_description!("[hour]:[minute]:[second]"))?
        } else {
            time!(12:00:00)
        };

        let lap_start = PrimitiveDateTime::new(date, time).assume_utc().into();

        let lap_type = match entry_dict.get("type") {
            Some(val) => sport_type_map.get(val.as_str()).map(|_| val.into()),
            None => None,
        };

        let lap_number: i32 = match entry_dict.get("lap") {
            Some(v) => v.parse()?,
            None => -1,
        };

        let lap_dur: f64 = match entry_dict.get("dur") {
            Some(v) => convert_time_string(v)?,
            None => 0.,
        };

        let lap_dis: f64 = match entry_dict.get("dis") {
            Some(v) => {
                if v.contains("mi") {
                    let dis: f64 = v
                        .split("mi")
                        .next()
                        .ok_or_else(|| format_err!("shouldn't be possible"))?
                        .parse()?;
                    dis * METERS_PER_MILE
                } else if v.contains('m') {
                    v.split('m')
                        .next()
                        .ok_or_else(|| format_err!("shouldn't be possible"))?
                        .parse()?
                } else {
                    v.parse()?
                }
            }
            None => 0.,
        };

        let lap_cal: i32 = match entry_dict.get("cal") {
            Some(v) => v.parse()?,
            None => 0,
        };

        let lap_avghr: Option<f64> = match entry_dict.get("avghr") {
            Some(v) => Some(v.parse()?),
            None => None,
        };

        Ok(GarminLap {
            lap_type,
            lap_index: -1,
            lap_start,
            lap_duration: lap_dur,
            lap_distance: lap_dis,
            lap_trigger: None,
            lap_max_speed: None,
            lap_calories: lap_cal,
            lap_avg_hr: lap_avghr,
            lap_max_hr: None,
            lap_intensity: None,
            lap_number,
            lap_start_string: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use approx::assert_abs_diff_eq;
    use std::path::Path;

    use garmin_lib::date_time_wrapper::iso8601::convert_datetime_to_str;
    use garmin_models::garmin_correction_lap::GarminCorrectionLap;
    use garmin_utils::sport_types::SportTypes;

    use crate::{garmin_parse::GarminParseTrait, garmin_parse_txt};

    #[test]
    fn test_garmin_parse_txt() -> Result<(), Error> {
        let corr_map =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")?;
        let gfile = garmin_parse_txt::GarminParseTxt::new()
            .with_file(&Path::new("../tests/data/test.txt"), &corr_map)
            .unwrap();
        assert_eq!(gfile.filename.as_str(), "test.txt");
        assert_eq!(gfile.sport, SportTypes::Elliptical);
        assert_eq!(gfile.filetype.as_str(), "txt");
        assert_eq!(
            convert_datetime_to_str(gfile.begin_datetime.into()),
            "2013-01-16T13:30:00Z"
        );
        assert_eq!(gfile.total_calories, 2700);
        assert_eq!(gfile.laps.first().unwrap().lap_index, 0);
        assert_eq!(gfile.laps.get(1).unwrap().lap_index, 1);
        assert_eq!(gfile.laps.len(), 3);
        assert_eq!(gfile.points.len(), 3);
        assert_abs_diff_eq!(gfile.total_distance, 17702.784);
        assert_abs_diff_eq!(gfile.total_duration, 6600.0);
        assert_abs_diff_eq!(gfile.total_hr_dur, 1881000.0);
        assert_abs_diff_eq!(gfile.total_hr_dis, 6600.0);
        Ok(())
    }

    #[test]
    fn test_garmin_parse_txt_default_time() -> Result<(), Error> {
        let corr_map =
            GarminCorrectionLap::corr_list_from_json("../tests/data/garmin_corrections.json")
                .unwrap();
        let gfile = garmin_parse_txt::GarminParseTxt::new()
            .with_file(&Path::new("../tests/data/test2.txt"), &corr_map)
            .unwrap();
        assert_eq!(
            convert_datetime_to_str(gfile.begin_datetime.into()),
            "2013-01-17T12:00:00Z"
        );
        Ok(())
    }
}
