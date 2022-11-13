.PHONY : executables
executables : target/release/storm_grok target/release/sg_server

dist/index.html : frontend/src/*
	cd frontend && trunk build --release

target/release/storm_grok : client/src/* dist/index.html
	cargo build --profile release-small --bin storm_grok

target/release/sg_server : server/src/*
# 	export RUSTFLAGS="-C link-args=-fuse-ld=lld" && cargo build --release --bin sg_server
	cargo build --release --bin sg_server

.PHONY : run run_server run_client
run :
	make -j 2 run_server run_client
run_server :
	cargo run --bin sg_server
run_client :
	cargo run --bin storm_grok http 4040 -d

.PHONY : test
test :
	cargo test
