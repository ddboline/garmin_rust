version := "0.1.22"
release := "1"
uniq := $(shell head -c1000 /dev/urandom | sha512sum | head -c 12 ; echo ;)
cidfile := "/tmp/.tmp.docker.$(uniq)"
build_type := release

all:
	mkdir -p build/ && \
	cp Dockerfile.ubuntu18.04 build/Dockerfile && \
	cp -a Cargo.toml src scripts Makefile python build/ && \
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

lambda_build:
	docker run --cidfile $(cidfile) -v `pwd`/target:/garmin_rust/target garmin_rust/build_rust:amazonlinux2018.03 /garmin_rust/scripts/build_lambda.sh
	docker cp `cat $(cidfile)`:/garmin_rust/rust.zip .
	docker rm `cat $(cidfile)`
	rm $(cidfile)

lambda_upload:
	aws s3 cp rust.zip s3://garmin-scripts-lambda-code/

lambda_create:
	aws cloudformation create-stack --stack-name garmin-rust-lambda --template-body file:///home/ddboline/setup_files/build/garmin_rust/cloudformation-templates/garmin_rust_lambda.json

lambda_update:
	aws cloudformation update-stack --stack-name garmin-rust-lambda --template-body file:///home/ddboline/setup_files/build/garmin_rust/cloudformation-templates/garmin_rust_lambda.json

lambda_update_code:
	aws lambda update-function-code --function-name garmin_rust_lambda --s3-bucket garmin-scripts-lambda-code --s3-key rust.zip

install:
	cp target/$(build_type)/garmin-rust-proc /usr/bin/garmin-rust-proc
	cp target/$(build_type)/garmin-rust-report /usr/bin/garmin-rust-report
	cp target/$(build_type)/garmin-rust-http /usr/bin/garmin-rust-http
	cp python/strava_upload.py /usr/bin/strava-upload
	cp python/fitbit_auth.py /usr/bin/fitbit-auth

pull:
	`aws ecr get-login --no-include-email`
	docker pull 281914939654.dkr.ecr.us-east-1.amazonaws.com/rust_stable:latest
	docker tag 281914939654.dkr.ecr.us-east-1.amazonaws.com/rust_stable:latest rust_stable:latest
	docker rmi 281914939654.dkr.ecr.us-east-1.amazonaws.com/rust_stable:latest

dev:
	docker run -it --rm -v `pwd`:/garmin_rust rust_stable:latest /bin/bash || true
