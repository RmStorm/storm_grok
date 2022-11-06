# storm_grok_server
Homegrown ngrok clone, written in rust! (server)

The client is over at https://github.com/RmStorm/storm_grok

## Development

Simply run `cargo run` and you're off to the races! In dev mode self signed certificates for localhost are used.

## Deployment

This project is meant for you to run your own fully certified copy of stormgrok :tm:. It's sligthly more involved then just running locally. You will need some server out on the internet and you will need to get certificates. But that is really it! What follows is a write up of how I currently have this running at `*.stormgrok.nl`.

First build a binary using `cargo build --release`. Then copy the binary and the config directory to your favourite server. I've been using a small Dutch web provider for domain names called [transip](https://www.transip.nl) for a long time. [Certbot](https://certbot.eff.org/) did not support that provider well so I'm using [Lego](https://go-acme.github.io/lego/) instead to get my certs.

### certs

Lego is quite nice! You need to run this command by hand once in the root directory of `my-user` at `/home/my-user`. See their list of supported dns providers [here](https://go-acme.github.io/lego/dns/), it's quite extensive! 

```bash
TRANSIP_ACCOUNT_NAME="my-transip-user" TRANSIP_PRIVATE_KEY_PATH="/path/to/my/key" lego -m "myemail@gmail.com" --dns transip -d *.stormgrok.nl -d stormgrok.nl run
```

After that you can add the following command to crontab using `sudo crontab -e`.

```bash
@weekly export TRANSIP_ACCOUNT_NAME="my-transip-user" TRANSIP_PRIVATE_KEY_PATH="/path/to/my/key"; cd /home/my-user/; lego -m "myemail@gmail.com" --dns transip -d *.stormgrok.nl -d stormgrok.nl renew >> /var/log/cron.log 2>&1
```

The logs will be here:

```bash
sudo cat /var/log/cron.log
```

example logs of postponed automatic renewal:

```
Oct 23 14:23:01 ubuntu-2gb-nbg1-1-VM1 CRON[151218]: (root) CMD (export TRANSIP_ACCOUNT_NAME="my-transip-user" TRANSIP_PRIVATE_KEY_PATH="/path/to/my/key"; cd /home/my-user/; lego -m "myemail@gmail.com" --dns transip -d *.stormgrok.nl -d stormgrok.nl renew >> /var/log/cron.log 2>&1)
2022/10/23 14:23:02 [*.stormgrok.nl] The certificate expires in 89 days, the number of days defined to perform the renewal is 30: no renewal.
```

This results in valid certificates being stored in `/home/my-user/.lego/certificates` which are automatically renewed. These can be supplied to the stormgrok binary using environment variables.

### Direct Invocation

Now that you have certificates you can start stormgrok. I have placed both the binary and the `config` directory in a folder called `sg_server`. The whole thing ends up looking like this:

```
/home/my-user
└───sg_server
│   │   sg_server
│   └───config
│       │   Default.toml
│       │   Prod.toml
└───.lego
    └───certificates
        │   _.stormgrok.nl.crt
        │   _.stormgrok.nl.key
```

Starting it can be done like so:

```bash
cd sg_server
sudo SG_SERVER__TLS__CERT_FILE=/home/my-user/.lego/certificates/_.stormgrok.nl.crt SG_SERVER__TLS__KEY_FILE=/home/my-user/.lego/certificates/_.stormgrok.nl.key RUN_ENV=Prod ./sg_server
```

### Systemd

All of this is off course easy to wrap in a systemd service like so:

```bash
cat /etc/systemd/system/storm_grok_server.service
```

```ini
[Unit]
Description=https://github.com/RmStorm/storm_grok_server/
Wants=network-online.target
After=network-online.target

[Service]
Environment="SG_SERVER__TLS__CERT_FILE=/home/my-user/.lego/certificates/_.stormgrok.nl.crt"
Environment="SG_SERVER__TLS__KEY_FILE=/home/my-user/.lego/certificates/_.stormgrok.nl.key"
Environment="RUN_ENV=Prod"
User=root
WorkingDirectory=/home/my-user/sg_server
ExecStart=/home/my-user/sg_server/sg_server
Restart=always
RestartSec=1

[Install]
WantedBy=multi-user.target
```

Start it using:

```bash
sudo systemctl start storm_grok_server.service
```
