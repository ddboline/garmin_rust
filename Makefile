version := "0.1.12"
release := "1"
uniq := $(shell head -c1000 /dev/urandom | sha512sum | head -c 12 ; echo ;)
cidfile := "/tmp/.tmp.docker.$(uniq)"
build_type := release

all:
	cp Dockerfile.ubuntu18.04 Dockerfile
	docker build -t build_rust:ubuntu18.04 .
	rm Dockerfile

amazon:
	cp Dockerfile.amazonlinux2018.03 Dockerfile
	docker build -t build_rust:amazonlinux2018.03 .
	rm Dockerfile

cleanup:
	docker rmi `docker images | python -c "import sys; print('\n'.join(l.split()[2] for l in sys.stdin if '<none>' in l))"`
	rm -rf /tmp/.tmp.docker.garmin_rust
	rm Dockerfile

package:
	docker run --cidfile $(cidfile) -v `pwd`/target:/garmin_rust/target build_rust:ubuntu18.04 /garmin_rust/scripts/build_deb_docker.sh $(version) $(release)
	docker cp `cat $(cidfile)`:/garmin_rust/garmin-rust_$(version)-$(release)_amd64.deb .
	docker rm `cat $(cidfile)`
	rm $(cidfile)

lambda_build:
	docker run --cidfile $(cidfile) -v `pwd`/target:/garmin_rust/target build_rust:amazonlinux2018.03 /garmin_rust/scripts/build_lambda.sh
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
	cp target/$(build_type)/garmin_rust_proc target/$(build_type)/garmin_rust_report target/$(build_type)/garmin_rust_http /usr/bin/
	cp python/strava_upload.py /usr/bin/strava-upload
