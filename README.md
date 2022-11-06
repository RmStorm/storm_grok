# storm_grok
Homegrown ngrok clone, written in rust! (client)

The server is over at https://github.com/RmStorm/storm_grok_server

## Architecture

The client contains a small bundled frontend so there are two components in this repo, server and frontend. The server in this context is not to be confused with the actual storm_grok_server that is running somewhere on the internet! The server component in here is running locally in the client and is only exposed on the localhost. It can be used for request replaying and stuff!

## Development

Before the storm_grok client can run you need to build the frontend. It's a small wasm app made using sycamore.

``` bash
cd frontend
trunk build [--release]
```

This preps 3 files in a directory called 'dist' which are used when running the client, running the client is done from the root of the repo like so:

``` bash
cargo run --bin storm_grok http 4040 [-d]
```

The optional `-d` is a flag for running in development mode. Without this flag the client will try to connect to `stormgrok.nl` at `157.90.124.255`. These are hardcoded for now. With the `-d` flag set it will instead try to connect to `localhost` at `127.0.0.1`. Clone the [storm grok server](https://github.com/RmStorm/storm_grok_server) and run it using `cargo run` to have something to connect to.

For building a release run:

``` bash
cargo build --release --bin storm_grok
```
