# storm_grok
Homegrown ngrok clone, written in rust!

## Architecture

The client contains a small bundled frontend to allow for request inspection. The architecture diagram here tries to explain the flow of traffic when running sgrok in http or tcp mode.
![](sgrok.png)

## Development

The server runs in development mode by default. The client needs a flag to run in development mode. Run both like so:
``` bash
cargo run --bin sg_server
cargo leptos watch -- http 8000 -d
```

The optional `-d` flag on the client is for running in development mode. Without this flag the client will try to connect to `stormgrok.nl` at `157.90.124.255`. These values are hardcoded for now. With the `-d` flag set it will instead try to connect to `localhost` at `127.0.0.1`.

### TODOS
- update server packages, preferably switch to pingora just like in the client!
- continously stream trafficlog from client to a frontend if connected
- Make a much nicer UI
- Allow request replaying
