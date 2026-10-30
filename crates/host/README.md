# Host simulator

Runs the real `cahoots-hal` device logic on a laptop with stand-ins for the hardware:

| Hardware | Simulator |
|---|---|
| TRNG | OS CSPRNG (`getrandom`), behind the SP 800-90B health tests. Or `--seed N` for a reproducible demo. |
| Touch link (UART) | A Unix-domain socket to each neighbour |
| Pad region (NOR flash or RRAM) | `pad.bin`, 4 KiB erase unit, erase = overwrite zeros + sync |
| Ledger region | `ledger.bin`, 64 KiB of `0xFF`, 16-byte append-only records |
| 200x200 e-paper | `display.png`, re-rendered on every screen change |

A simulated badge is a `CSPRNG` source, not a hardware TRNG. Its circles are labelled `CSPRNG`.

## Four devices on one laptop

Each device listens on its own socket and connects to the next one, so every device has a socket to both neighbours. Frames travel both ways over every socket, as they do through the pads in a pile, so the ring may be closed or left open. Start all four within 30 seconds of each other.

```sh
cargo build -p cahoots-host
H=target/debug/cahoots-host
for i in 0 1 2 3; do
  $H ceremony --index $i --n 4 --pad-len 65536 --name sf-meetup \
    --listen /tmp/cahoots$i.sock --next /tmp/cahoots$(( (i + 1) % 4 )).sock \
    --state /tmp/cahoots-dev$i &
done
wait
```

Every device prints the same six check words and writes its state directory. Open `/tmp/cahoots-dev0/display.png` to see what the badge would show.

Send a message from device 0 and read it on device 3:

```sh
MSG=$($H send --state /tmp/cahoots-dev0 --text "hello from the ring")
$H recv --state /tmp/cahoots-dev3 --message "$MSG"
$H recv --state /tmp/cahoots-dev3 --message "$MSG"   # second time: refused as reuse
$H status --state /tmp/cahoots-dev0                   # spent ranges and remaining bytes
xxd /tmp/cahoots-dev0/pad.bin | head -4               # the spent range reads as zeros
```

The base64url message can be pasted into the browser app (Read a message) or posted to the Numbers Station. Any other device that holds the same pad decrypts it.

## What the integration test covers

`tests/simulator.rs` runs four devices in threads over socket pairs with deterministic entropy, checks that all four check words match the words computed directly from the XOR of the contributions, sends a message from device 0 to device 3, confirms both pad files show the spent range erased on disk and the rest intact, confirms a replay is refused, and reopens a device from disk to confirm the ledger replays.

## Ring driver notes

Writes on the socket link never block. Bytes that do not fit the socket buffer are queued and flushed on every later call. The ring driver keeps reading while it flushes its last forwarded frames before it finishes, so a device that completes first never strands its neighbour. A UART driver with a DMA ring buffer needs both behaviours too.

## A pile on the laptop, with a real board on top

```sh
cargo run -p cahoots-host -- pile --cards 3 --state /tmp/pile                       # three simulated cards
cargo run -p cahoots-host -- pile --cards 3 --serial /dev/cu.usbmodem* --state /tmp/pile   # two simulated, a board on top
cargo run -p cahoots-host -- logs --serial /dev/cu.usbmodem*                              # a board's log lines, nothing sent
```

`pile` stacks cards on Unix sockets, bottom first, and runs `hal::pile` discovery on each: the bottom card names the circle, the top card presses Both. With `--serial`, the top simulated card's up link is that serial port and a real badge is the top card; its buttons start the count. `SerialLink` mirrors `SocketLink` and adds what a USB port needs: a `Ping` frame twice a second so the board can tell the laptop is there, and the board's `Log` frames printed here as `[board] ...`.

## The badge crate on the laptop

`badge` runs one card through `cahoots-badge`, the same code a real board runs, on a `HostBoard`: the boot screen, then a pile pass while the card has no circle, then the idle menu. `--listen` is the socket the card below connects to (the down link), and `--next` is the card above's socket (the up link). Use `-` for a side with nothing plugged in.

```sh
H=target/debug/cahoots-host
$H badge --state /tmp/b0 --listen - --next /tmp/b1.sock --name sf --pad-len 16384 &   # bottom
$H badge --state /tmp/b1 --listen /tmp/b1.sock --next - --name sf --pad-len 16384 &   # top
echo B > /tmp/b1/buttons.txt     # top card, once it shows its seat: Both starts the count
echo B > /tmp/b0/buttons.txt     # at "keep this circle?" on each card: Both keeps it
echo B > /tmp/b1/buttons.txt
```

The state directory holds the files `Device` keeps (`pad.bin`, `ledger.bin`, `circle.txt`), so `status`, `send` and `recv` work on it.

`circle.txt` has one `key=value` per line: `params`, `index`, `circle_id`, `words` and `label`, then `seat_key` (this seat's signing key, in hex), `tier` (the tier the card declared, 0 on a laptop) and `manifest` (the sealed manifest every seat signed, in hex). The seat key sits in the file in plaintext, as the pad sits in `pad.bin`; a laptop is a tier 0 device and promises no more. A `circle.txt` from before seat keys has none of the last three lines: `Device::open` still opens it, with no seat key and no manifest, and a `HostBoard` treats it as no circle.

The state directory also holds:

| File | What it does |
|---|---|
| `display.png` | The screen |
| `screen.txt` | The text of the latest screen, one line per row, so a script can follow the card |
| `buttons.txt` | Each line `L`, `R` or `B` is one gesture: Left, Right or Both. The card reads the file, removes it, and takes one gesture per poll. |
| `a_down` | While this file exists, button A is held. Create it before boot to forget the circle. |
| `softchip.bin` | With `--chip soft`: the software chip's keys, written after every chip call so a restarted card keeps them. Keys in the clear, like the rest of the directory. |

`--chip soft` gives the card a software chip. Its ceremony makes the seat key in the chip, and `circle.txt` then holds zeros in place of the key and a `chip_slot=` line; `Device::seat_key` returns nothing for such a circle, so nothing on the laptop signs with it.

`tests/badge.rs` runs two `HostBoard`s in threads over a socket, presses Both by writing `buttons.txt`, and checks that both cards keep the same circle and that `Device::open` reads the same check words from each directory.

## SSH logins and git commits with a badge's seat key

The laptop never holds the seat key. An SSH agent, `cahoots-host agent`, offers each circle's seat key to `ssh` and `git`, and sends every signature request to the badge, which shows what it is asked to sign and signs only when both buttons are pressed. The agent keeps no signature: each request is one question on the badge.

Build both programs and put them on your `PATH`:

```sh
cargo build --release -p cahoots-host
cp target/release/cahoots-host target/release/cahoots-host-git-sign ~/.local/bin/
```

### Start the agent

Plug the badge in. The first thing the laptop asks it makes the badge ask you, on its screen, whether to allow this laptop until it is unplugged; both buttons say yes. After that it answers from any page of its menu, and each signature still asks on its own. Then:

```sh
eval "$(cahoots-host agent --serial /dev/cu.usbmodem1101 --circle sf)"   # a real board
eval "$(cahoots-host agent --sim /tmp/b1.sock --circle sf)"              # a simulated badge, --listen /tmp/b1.sock
```

The agent prints `SSH_AUTH_SOCK=...; export SSH_AUTH_SOCK;` and keeps running; `eval` sets the variable in your shell. Give `--circle` once for each circle whose key you want. `--socket <path>` picks the socket (default `$TMPDIR/cahoots-agent.sock`). `--known-hosts <path>` picks the file host names come from (default `~/.ssh/known_hosts`).

```sh
ssh-add -L
# ecdsa-sha2-nistp256 AAAAE2VjZHNh... cahoots:sf:1
```

The comment is `cahoots:<circle>:<seat>`.

### `ssh -T git@github.com`

1. Copy the line `ssh-add -L` prints and add it to GitHub as an SSH key (Settings, SSH and GPG keys, New SSH key). Add it a second time as a signing key for verified commits.
2. With `SSH_AUTH_SOCK` set as above, run `ssh -T git@github.com`.
3. The badge shows "ssh login", "as git" and "to github.com (laptop)". Press both buttons.
4. GitHub answers `Hi <you>! You've successfully authenticated`.

The host name comes from the laptop: OpenSSH tells the agent the server's host key, and the agent finds its name in `known_hosts`. The badge checks that the server signed this session with that key (Ed25519 and P-256 host keys) and marks the name "(laptop)", because only `known_hosts` says which name the key belongs to. A hashed `known_hosts` entry keeps no name, so the agent tries the `Host` and `HostName` values in the `config` file beside `known_hosts` and a few git hosts (github.com, gitlab.com, bitbucket.org, codeberg.org, ssh.dev.azure.com). With no name, or a key the badge cannot check (RSA), the badge shows "host not named".

Press A or B alone to refuse: `ssh` gets a failure and tries its other keys.

### Signing git commits

git runs `gpg.ssh.program` with arguments (`-Y sign -n git -f <key file> <file>`), so the program must be one executable: `cahoots-host-git-sign`, built beside `cahoots-host`. (`cahoots-host git-sign` does the same from a shell.) It sends the commit text to the badge through the agent, so the badge shows the commit's author and subject and checks that the text is what it signs. The five settings:

```sh
git config --global gpg.format ssh
git config --global user.signingkey "key::$(ssh-add -L | grep ' cahoots:sf:')"
git config --global gpg.ssh.program cahoots-host-git-sign
git config --global commit.gpgsign true
git config --global gpg.ssh.allowedSignersFile ~/.ssh/allowed_signers
```

`key::` matters: git reads a `user.signingkey` that does not start with `ssh-` as a file path, and a seat key starts with `ecdsa-`. The last line is for checking signatures on this laptop; add a line `you@example.com namespaces="git" ecdsa-sha2-nistp256 AAAA...` to that file. `git log --show-signature` runs `cahoots-host-git-sign -Y verify ...`, which hands the check to `ssh-keygen`.

`git commit` now waits for the badge: it shows "sign a git commit", the author and the subject. Both buttons sign; A or B alone and git stops with "failed to write commit object". The badge signs only commits: a signed tag, or a commit text longer than the 16 KiB a badge reads, is refused.

`git-sign` finds the agent through `SSH_AUTH_SOCK` (or `CAHOOTS_AGENT_SOCK`, which wins when both are set), so the agent must be running.

### Signing a file

```sh
cahoots-host sign --file release.tar.gz --circle sf            # writes release.tar.gz.sig
cahoots-host sign --file notes.txt --circle sf --namespace notes
```

The signature is the SSH signature format (`-----BEGIN SSH SIGNATURE-----`), in the namespace `file` unless `--namespace` says otherwise, so anyone checks it with OpenSSH alone:

```sh
echo 'you@example.com namespaces="file" ecdsa-sha2-nistp256 AAAA...' > allowed_signers
ssh-keygen -Y verify -f allowed_signers -I you@example.com -n file -s release.tar.gz.sig < release.tar.gz
```

What the badge shows depends on the file's size. Up to 12 KiB, the file goes to the badge, which checks that the hash it is asked to sign is the hash of that file and shows "sign a file", the namespace, the size, and the file's first line (or "binary"). Bigger than that, the badge is sent only the hash, and shows it marked "(laptop)": it is signing what the laptop says the file is. `ssh-keygen -Y sign` through the agent does the same for any namespace but `git`, which still needs the commit text. A file signature can never be mistaken for a login or a commit: the signature names its namespace, and a verifier checks it.

### Grants for an agent

A grant lets a coding agent on this laptop sign git commits with the seat key without a press on the badge, as one author, a fixed number of times:

```sh
cahoots-host grant request --serial /dev/cu.usbmodem1101 --circle sf --agent "claude on my laptop" \
    --author "Ada Lovelace <ada@example.com>" --budget 50 --until 2026-11-29 [--location github.com/you/repo]
cahoots-host grant list
cahoots-host grant show ~/.config/cahoots/grants/sf-1a2b3c4d.cwt
cahoots-host grant verify ~/.config/cahoots/grants/sf-1a2b3c4d.cwt --state /tmp/b0 --circle sf
```

`grant request` makes the agent's own key the first time (`agent.key`, 32 secret bytes as hex, readable by you only), then asks the badge. The badge shows "let an agent sign", the agent's name marked "(laptop)", the author, how many signatures, and until when (`--until` is midnight UTC that day, or `--standing` for until revoked). Both buttons allow it. The grant is kept in `grants/` beside the key, in `$CAHOOTS_HOME` or `~/.config/cahoots`, and its path is printed.

Start the agent after that (or keep it running: it reads the grants when git signs). When git signs a commit through `cahoots-host-git-sign` with the circle's key, the agent sees the grant, checks that the commit's author is the grant's author, asks the badge for a fresh nonce, and sends the commit with the grant and a proof made with its key. The badge signs without asking and shows "signed for <agent>", the commit's subject and "37 left of 50". The agent's log says "signed under a grant, 37 left". Once the badge's answers have counted the grant down to 0, the agent stops using it and asks the ordinary way: the badge shows "sign a git commit" and waits for its holder. When the badge refuses a signature under the grant (it has expired or been revoked, or the badge fell back to asking and its holder refused), the signature fails and the agent's log says "refused on the badge, or the grant is no longer valid: run `cahoots-host grant list`". The agent leaves the grant as it is and asks nobody again. To sign the ordinary way while a grant is kept but no longer valid, delete its `.cwt` and `.json` files. `--location` limits the grant to one repository, compared with the `remote.origin.url` the helper reads. That is the laptop's word, and the weakest check.

`grant list` prints each kept grant with what the badge last said was left. `grant show` prints any grant's claims as JSON. `grant verify` checks the grant as any CWT verifier would, against the seat's key in the circle's manifest (read from `--state`, as `roster` reads it), with nothing from Cahoots but the manifest. The holder revokes a grant on the badge's grants page.

### A circle as `authorized_keys`

```sh
cahoots-host roster --ssh --circle sf --state /tmp/b1 >> ~/.ssh/authorized_keys
```

One line per seat, comment `cahoots:<circle>:<seat>`, from the circle's sealed manifest. The manifest comes from a card's state directory: a simulated badge's, or any simulated card's from a `pile` run with a real board on top, since every seat keeps the same manifest. A badge keeps its first circle in `circle.txt` and the others in `circle-<k>.txt`; `roster` and `card` find the circle by name in any of them. The badge's `Identify` answer names its circles but carries no manifest. Every seat's signature on the manifest is checked before anything is printed.

### Attesting a seat key

```sh
cahoots-host attest --serial /dev/cu.usbmodem1101 --circle sf [--test-root] [--json]
```

Asks the badge for the circle's seat key, sends a fresh random value, asks the badge's chip to sign the seat key with it, prints the answer as JSON, and checks it. The check passes only when the signature leads to a trusted root and the attested key is the seat key the badge named. It exits non-zero when the check fails. NXP's attestation root is not in this program yet (`crates/objects/roots/README.md`), so a real SE051 cannot pass, and `attest` says so. With `--sim`, or with `--test-root` on a board, the software chip's test root is also accepted; its private key is in the source, so a pass against it is printed as a software chip, not hardware, and the JSON has `"hardware": false`. A circle whose key is in software has nothing to attest.

Sharing an attestation shows which chip holds the key, so two circles that both see attestations from one badge can tell they share a member. The badge gives an attestation without asking its holder.

`attest` opens the badge's link itself; stop the agent first, since a board's port and a simulated badge's socket serve one program at a time.

### Contact cards

```sh
cahoots-host card set --serial /dev/cu.usbmodem1101 --circle sf --name "Ada" \
    --handle email=ada@example.com --handle github=ada --endpoint ssh=ada.example.com --out ada.card
cahoots-host card fetch --serial /dev/cu.usbmodem1101 --circle sf --state /tmp/b0 --in ada.card --in bob.card [--out cards/]
cahoots-host card show --circle sf --state /tmp/b0 cards/1.card
```

`card set` builds this seat's card, asks the badge to sign it (the badge shows "sign your card", the name, and how many handles and endpoints), then asks for the circle's card seal key (the badge shows "key for seal/card"). It writes the sealed card to `--out`. Endpoint purposes are `wireguard`, `tls`, `mls`, `wifi`, `meshtastic`, `other`, `ssh` and `chat`. `card fetch` asks for the seal key, opens each sealed card, checks its signature against the manifest in `--state` (as `roster` reads it), and prints it as JSON. With `--out`, it also writes each signed card there as `<seat>.card`, which `card show` checks and prints. Cards travel as files. Publishing them on the circle's station topic is not built yet. The laptop does not keep the seal key, so each fetch asks the badge again. The badge keeps the last card it signed for each circle in memory until it restarts.

### Grants

```sh
cahoots-host grant --serial /dev/cu.usbmodem1101 --circle sf --to <33-byte key hex> \
    --purpose wireguard --purpose tls --budget 4096 --until 1000 [--depth 1] [--v2] [--out grant.cose]
```

Builds a root grant from this seat to the key `--to`, asks the badge to sign it with the seat key, and writes the signed grant to `--out`, or as hex to stdout. The JSON form goes to stderr. With `--v2`, the payload is a CBOR Web Token (`docs/protocol.md` section 8.3). The badge shows the grantee's short key id and the log position marked "(laptop)", and shows the purposes, the budget and whether the grantee may delegate.

### The bench

With a real board on `--serial`, `CAHOOTS_BENCH=new-circle` or `CAHOOTS_BENCH=serve` in the environment makes the serial link press the badge's buttons for that goal, by reading the badge's screen lines, when the firmware was built with its `bench` feature. `CAHOOTS_FRAMES=1` prints every frame that crosses the port and dumps the bytes received to `/tmp/cahoots-rx-<label>.bin`. `scripts/bench.sh` puts it together; see the [Pico 2 README](../rp2350/README.md#the-bench-hands-off).

### Tests

`tests/agent.rs` runs a ceremony between two simulated badges with software chips, serves the top one, starts the agent as a process, and checks the identity, a signature that verifies, a refusal, an unknown key, and the host named from `session-bind@openssh.com` and `known_hosts`. `tests/gitsign.rs` signs a commit text the way git calls `gpg.ssh.program` and checks the result with `ssh-key` and `ssh-keygen -Y verify`. `tests/roster.rs` checks the `authorized_keys` lines against the manifest and finds a circle kept in `circle-2.txt`, `tests/attest.rs` the attestation of a software chip, and `tests/card.rs` a card set, fetched and shown, and a version 2 grant that verifies against the manifest.
