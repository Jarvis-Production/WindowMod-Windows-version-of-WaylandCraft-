#!/usr/bin/env sh

cd native
cargo build --target-dir target "$@"
cd ..
./gradlew build
