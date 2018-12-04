#[cfg(test)]
mod tests {
    #[test]
    fn test_corr_list_from_json() {
        let corr_list = garmin_rust::garmin_correction_lap::corr_list_from_json(
            "tests/data/garmin_corrections.json",
        ).unwrap();

        assert_eq!(corr_list.get(0).unwrap().distance, Some(3.10685596118667));

        let corr_val = garmin_rust::garmin_correction_lap::GarminCorrectionLap::new();
        assert_eq!(corr_val.lap_number, -1);
    }
}
