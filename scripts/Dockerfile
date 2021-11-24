FROM rust:alpine3.14 as forerunner

RUN apk add musl-dev git gcc make libusb-dev libusb-compat-dev

RUN mkdir -p /build /garmin-forerunner-tools && git clone https://github.com/rogg/garmin-forerunner-tools.git /build/garmin-forerunner-tools

RUN cd /build/garmin-forerunner-tools && \
    ./configure --prefix=/garmin-forerunner-tools && \
    make && make install

FROM rust:alpine3.14 as buildstage

RUN apk add musl-dev

RUN mkdir -p /build/garmin_rust

ADD Cargo.toml /build/garmin_rust/
COPY src /build/garmin_rust/src
COPY fitbit_bot /build/garmin_rust/fitbit_bot
COPY fitbit_lib /build/garmin_rust/fitbit_lib
COPY garmin_cli /build/garmin_rust/garmin_cli
COPY garmin_connect_lib /build/garmin_rust/garmin_connect_lib
COPY garmin_http /build/garmin_rust/garmin_http
COPY garmin_lib /build/garmin_rust/garmin_lib
COPY garmin_reports /build/garmin_rust/garmin_reports
COPY race_result_analysis /build/garmin_rust/race_result_analysis
COPY strava_lib /build/garmin_rust/strava_lib
COPY migrations /build/garmin_rust/migrations
COPY templates /build/garmin_rust/templates

RUN mkdir -p /garmin_rust && \
    cd /build/garmin_rust && \
    cargo install --path=. --root=/garmin_rust

FROM alpine:3.14

COPY --from=forerunner /garmin-forerunner-tools/* /usr/
COPY --from=buildstage /garmin_rust/bin/* /usr/bin/

RUN apk add openssh-client

STOPSIGNAL SIGTERM

EXPOSE 3042

CMD ["/usr/bin/garmin-rust-http"]
