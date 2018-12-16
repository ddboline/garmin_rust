#!/bin/bash

. ~/.cargo/env

cargo build --release --bin bootstrap --target x86_64-unknown-linux-musl

mkdir tmp

cp target/release/bootstrap tmp/
cp -a lambda/bin lambda/lib tmp/

cd tmp/

zip --symlinks ../rust.zip bootstrap bin/* lib/*

cd ../

rm -rf tmp/

