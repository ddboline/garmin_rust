#[cfg(test)]
mod tests {
    use garmin_rust::common::garmin_cli;

    #[test]
    fn test_garmin_cli_new() {
        let gcli = garmin_cli::GarminCli::new();
        assert_eq!(gcli.do_sync, false);
        assert_eq!(gcli.do_all, false);
        assert_eq!(gcli.do_bootstrap, false);
        assert_eq!(gcli.filenames, None);
    }

    #[test]
    fn test_garmin_file_test_avro() {
        
    }
}
