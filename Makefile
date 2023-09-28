.PHONY : executables
executables : target-x86/release/storm_grok target-x86/release/sg_server

dist/index.html : frontend/*
	cd frontend && trunk build --release

target-x86/release/storm_grok : client/src/* dist/index.html
	cargo build --target-dir target-x86 --profile release-small --bin storm_grok

target-x86/release/sg_server : server/src/*
	cargo build --target-dir target-x86 --release --bin sg_server

.PHONY : run run_server run_client
run :
	make -j 2 run_server run_client
run_server :
	cargo run --target-dir target-x86 --bin sg_server
run_client :
	cargo run --target-dir target-x86 --bin storm_grok http 4040 -d

.PHONY : test
test :
	cargo test --target-dir target-x86
