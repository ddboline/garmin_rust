version = $(shell awk '/^version/' Cargo.toml | head -n1 | cut -d "=" -f 2 | sed 's: ::g')
release := "1"
uniq := $(shell head -c1000 /dev/urandom | sha512sum | head -c 12 ; echo ;)
cidfile := "/tmp/.tmp.docker.$(uniq)"
build_type := release

all:
	mkdir -p build/ && \
	cp Dockerfile.build.ubuntu18.04 build/Dockerfile && \
	cp -a Cargo.toml src scripts Makefile python templates build/ && \
	cd build/ && \
	docker build -t garmin_rust/build_rust:ubuntu18.04 . && \
	cd ../ && \
	rm -rf build/

amazon:
	cp Dockerfile.amazonlinux2018.03 Dockerfile
	docker build -t garmin_rust/build_rust:amazonlinux2018.03 .
	rm Dockerfile

cleanup:
	docker rmi `docker images | python -c "import sys; print('\n'.join(l.split()[2] for l in sys.stdin if '<none>' in l))"`
	rm -rf /tmp/.tmp.docker.garmin_rust
	rm Dockerfile

package:
	docker run --cidfile $(cidfile) -v `pwd`/target:/garmin_rust/target garmin_rust/build_rust:ubuntu18.04 \
        /garmin_rust/scripts/build_deb_docker.sh $(version) $(release)
	docker cp `cat $(cidfile)`:/garmin_rust/garmin-rust_$(version)-$(release)_amd64.deb .
	docker rm `cat $(cidfile)`
	rm $(cidfile)

test:
	docker run --cidfile $(cidfile) -v `pwd`/target:/garmin_rust/target garmin_rust/build_rust:ubuntu18.04 /bin/bash -c ". ~/.cargo/env && cargo test"

build_test:
	cp Dockerfile.test.ubuntu18.04 build/Dockerfile && \
	cd build/ && \
	docker build -t garmin_rust/test_rust:ubuntu18.04 . && \
	cd ../ && \
	rm -rf build/

install:
	cp target/$(build_type)/garmin-rust-proc /usr/bin/garmin-rust-proc
	cp target/$(build_type)/garmin-rust-report /usr/bin/garmin-rust-report
	cp target/$(build_type)/garmin-rust-http /usr/bin/garmin-rust-http
	cp python/strava_upload.py /usr/bin/strava-upload
	cp python/fitbit_auth.py /usr/bin/fitbit-auth

pull:
	`aws ecr --region us-east-1 get-login --no-include-email`
	docker pull 281914939654.dkr.ecr.us-east-1.amazonaws.com/rust_stable:latest
	docker tag 281914939654.dkr.ecr.us-east-1.amazonaws.com/rust_stable:latest rust_stable:latest
	docker rmi 281914939654.dkr.ecr.us-east-1.amazonaws.com/rust_stable:latest

dev:
	docker run -it --rm -v `pwd`:/garmin_rust rust_stable:latest /bin/bash || true

get_version:
	echo $(version)
