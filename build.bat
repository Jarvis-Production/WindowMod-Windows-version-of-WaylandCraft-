@echo off
REM Build the native library in RELEASE mode. The jar task and `gradlew
REM runClient` both load native/target/release/waylandcraft.dll (release has
REM priority). Building only debug left a STALE release .dll in place, so every
REM code change silently did nothing in-game. Always build --release here.
cd native
cargo build --release --target-dir target %*
cd ..
REM Force the jar to repack: Gradle does not track the native .dll as an input,
REM so without --rerun-tasks it reports the jar "up-to-date" and ships the OLD
REM embedded library even after the .dll was rebuilt.
call gradlew.bat build --rerun-tasks %*

