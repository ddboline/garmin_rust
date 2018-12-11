#!/bin/bash

VERSION="$1"
RELEASE="$2"

source ~/.cargo/env

cargo build --release
printf "\ninstall:\n\tcp target/release/garmin_rust_proc target/release/garmin_rust_report target/release/garmin_rust_http /usr/bin/\n" > Makefile
printf "Process and display info about gps activity files\n" > description-pak
checkinstall --pkgversion ${VERSION} --pkgrelease ${RELEASE} -y
