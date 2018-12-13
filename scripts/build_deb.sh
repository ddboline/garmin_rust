#!/bin/bash

VERSION="$1"
RELEASE="$2"

printf "Process and display info about gps activity files\n" > description-pak
checkinstall --pkgversion ${VERSION} --pkgrelease ${RELEASE} -y
