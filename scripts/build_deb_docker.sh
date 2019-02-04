#!/bin/bash

VERSION="$1"
RELEASE="$2"

. ~/.cargo/env

cargo build --release

printf "Process and display info about gps activity files\n" > description-pak
echo checkinstall --pkgversion ${VERSION} --pkgrelease ${RELEASE} -y
checkinstall --pkgversion ${VERSION} --pkgrelease ${RELEASE} -y
