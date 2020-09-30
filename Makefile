version = $(shell awk '/^version/' Cargo.toml | head -n1 | cut -d "=" -f 2 | sed 's: ::g')
release := "1"
uniq := $(shell head -c1000 /dev/urandom | sha512sum | head -c 12 ; echo ;)
cidfile := "/tmp/.tmp.docker.$(uniq)"
build_type := release

all:
	mkdir -p build/ && \
	cp Dockerfile.build.ubuntu20.04 build/Dockerfile && \
	cp -a Cargo.toml src scripts Makefile templates garmin_cli \
		garmin_lib garmin_http fitbit_lib fitbit_bot strava_lib \
		garmin_connect_lib race_result_analysis garmin_reports build/ && \
	cd build/ && \
	docker build -t garmin_rust/build_rust:ubuntu20.04 . && \
	cd ../ && \
	rm -rf build/

cleanup:
	docker rmi `docker images | python -c "import sys; print('\n'.join(l.split()[2] for l in sys.stdin if '<none>' in l))"`
	rm -rf /tmp/.tmp.docker.garmin_rust
	rm Dockerfile

package:
	docker run --cidfile $(cidfile) -v `pwd`/target:/garmin_rust/target garmin_rust/build_rust:ubuntu20.04 \
        /garmin_rust/scripts/build_deb_docker.sh $(version) $(release)
	docker cp `cat $(cidfile)`:/garmin_rust/garmin-rust_$(version)-$(release)_amd64.deb .
	docker rm `cat $(cidfile)`
	rm $(cidfile)

test:
	docker run --cidfile $(cidfile) -v `pwd`/target:/garmin_rust/target garmin_rust/build_rust:ubuntu20.04 /bin/bash -c ". ~/.cargo/env && cargo test"

build_test:
	cp Dockerfile.test.ubuntu20.04 build/Dockerfile && \
	cd build/ && \
	docker build -t garmin_rust/test_rust:ubuntu20.04 . && \
	cd ../ && \
	rm -rf build/

install:
	cp target/$(build_type)/garmin-rust-cli /usr/bin/garmin-rust-cli
	cp target/$(build_type)/garmin-rust-http /usr/bin/garmin-rust-http
	cp target/$(build_type)/scale-measurement-bot /usr/bin/scale-measurement-bot
	cp target/$(build_type)/import-garmin-connect-data /usr/bin/import-garmin-connect-data
	cp target/$(build_type)/import-fitbit-json-files /usr/bin/import-fitbit-json-files

pull:
	`aws ecr --region us-east-1 get-login --no-include-email`
	docker pull 281914939654.dkr.ecr.us-east-1.amazonaws.com/rust_stable:latest
	docker tag 281914939654.dkr.ecr.us-east-1.amazonaws.com/rust_stable:latest rust_stable:latest
	docker rmi 281914939654.dkr.ecr.us-east-1.amazonaws.com/rust_stable:latest

dev:
	docker run -it --rm -v `pwd`:/garmin_rust rust_stable:latest /bin/bash || true

get_version:
	echo $(version)
