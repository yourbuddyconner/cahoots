# Deploying the default relay

The browser app ships with `https://relay.cahoots.network` as its relay, so a new circle chats without anyone running a server. This folder is the recipe for that relay. Any host that runs a container with a persistent disk and HTTPS in front works; the file here is for [Fly.io](https://fly.io), which gives all three with one config.

The relay holds no keys and no membership. It stores ciphertext by circle and serves it back. What it needs is what any public endpoint needs: HTTPS, a disk that survives restarts, a health check, a rate limit, and a size cap.

## What the station provides itself

| Need | How |
|---|---|
| Rate limit | Per client address, in process: `--posts-per-minute` (default 120) and `--reads-per-minute` (default 600), each a burst of that size then that rate. Over the limit answers `429 {"error":"rate"}` with `Retry-After: 10`. |
| Client address behind a proxy | `--trust-forwarded` reads `Fly-Client-IP`, then the first `X-Forwarded-For` entry. Set it only behind a proxy that sets those headers. |
| Size cap | `--max-log-bytes N` refuses posts with `507 {"error":"full"}` once the log file is that large. Reads keep working. The log is never truncated: it is a hash chain, and cutting it would break every reader's view of history. |
| Health | `GET /healthz` answers `ok`. |
| Retention | Everything accepted is kept until the operator deletes the file. There is no per-message expiry. Members' devices keep their own copies, so losing the relay loses nothing a member already read. |

## Fly.io

Once, from the repository root, with `flyctl` signed in:

```sh
fly launch --config deploy/relay/fly.toml --copy-config --no-deploy --name cahoots-relay --region sjc
fly volumes create station_data --size 1 --region sjc --app cahoots-relay
fly deploy --config deploy/relay/fly.toml
fly certs add relay.cahoots.network --app cahoots-relay
```

Then add the DNS records `fly certs add` prints (an `A` and an `AAAA`, or a `CNAME`, for `relay.cahoots.network`). Check:

```sh
curl https://relay.cahoots.network/healthz
curl https://relay.cahoots.network/v1/head
```

The first answers `ok`; the second answers `{"seq":null,"hash":"…"}` on an empty log.

Updates: `fly deploy --config deploy/relay/fly.toml` from a checkout of `main`. The log on the volume is replayed and chain-verified on every start; a broken file refuses to start, which is the point.

## Any other host

```sh
docker build -f crates/station/Dockerfile -t cahoots-station .
docker run -d --restart unless-stopped -p 127.0.0.1:8080:8080 -v station-data:/data cahoots-station \
  --listen 0.0.0.0:8080 --log /data/station.jsonl --trust-forwarded --max-log-bytes 4294967296
```

Put a reverse proxy with HTTPS in front (Caddy: `relay.cahoots.network { reverse_proxy 127.0.0.1:8080 }`) and make sure it sets `X-Forwarded-For`.

## What this is not

This is a plain host, not the enclave deployment in the roadmap (C5). The operator can read the log (ciphertext) and could show different readers different histories; the hash chain lets a reader who kept an earlier head notice a rewrite, and nothing more. Members' devices are the source of truth for their own messages.
