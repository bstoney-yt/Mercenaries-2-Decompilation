# Online co-op ("the beta")

Two players, each with their own installed copy of the game, play the
island together over the internet. The host sends a friend one link; the
friend downloads the beta, runs it, and joins. As in the original's online
co-op, it is two players, not split screen.

Everything here is invented for this project: the original PC game's own
online code is not read or reused.

## What the download holds

Only our code: `m2-alpha.exe` (shipped as the beta) and its readme. No file
from the game. Each player's game reads the island, models, sounds and
scripts from that player's own install, found as `m2-alpha` already finds
it (below). Nothing from the game's files ever crosses the network either:
the messages carry positions, ids and numbers (see `m2-net/src/proto.rs`).

## How the two games talk (`m2-net`)

- **Transport:** [iroh](https://iroh.computer) (QUIC). Each game is an
  endpoint named by a public key. iroh punches through both routers when
  it can (no port forwarding), and otherwise passes the encrypted packets
  through a relay server. Measured on the test PC (round 1, two games on
  one machine): joined in 0.5 s; forced through the public relay, round
  trip 82 to 85 ms and 31 updates a second received of 30 sent.
- **Relay:** n0's public relays (free, no account; meant for development
  and small use). If they ever get in the way, a relay of our own
  (`iroh-relay`, one small server) can replace them by a setting. That
  needs a server and is the host's decision, not taken yet.
- **Channels:** one reliable stream for the handshake and events (a `u16`
  length, then the message), and QUIC datagrams for snapshots (latest
  wins; a lost one is replaced by the next).
- **Who decides what:** each player is the authority for their own hero
  (where they are, how they move, what they shoot). The host is the
  authority for the world: people, vehicles, the contract and its
  scripts. The friend's game shows the host's world and sends its
  requests (a hit, getting in a vehicle) to the host, which applies them.
  That is how the original's online co-op felt (each player's own hero
  responds at once) and it keeps one copy of the scripts running.
- **Playback:** the other hero is shown 100 ms behind its newest
  snapshot, moving in a straight line between snapshots, and carries on
  for at most 250 ms if they stop coming (`m2-net/src/interp.rs`).

## The invite link

```text
https://bstoney-yt.github.io/Mercenaries-2-Decompilation/join/#<code>
<code> = <host's endpoint id>@<relay host>
```

The code rides in the link's fragment, which browsers never send to the
web server. The page (a static GitHub Pages page, stage 4):

1. says what this is and that the friend needs their own copy of
   Mercenaries 2 installed;
2. has one button that copies the code and downloads the latest beta
   from the repository's GitHub Releases (`releases/latest/download/...`);
3. explains: unzip, run, and the beta offers to join (it reads the code
   from the clipboard; pasting it into the join box works too).

Later, the beta can register an `m2beta:` link handler for the current
user, so a second invite opens the game directly.

Planned for stage 4: the host's key is kept between sessions so the same
link keeps working, and since anyone with the link can ask to join, the
host's game asks the host to accept a new player. (Stage 1 makes a new
key each session and lets the first player in.)

## Finding the friend's install

In order: `--install` or the settings' `install`; the usual Program Files
folders (`EA Games`, `Origin Games`); the installer's registry value
`HKLM\SOFTWARE\WOW6432Node\EA Games\Mercenaries 2 World in Flames\Install Dir`
(seen on one real install); then (stage 4) a folder picker, remembered in
the settings.

## Stages

Each ends with numbers from a run.

1. **Two games see each other's hero** (this round). `m2-alpha --host` or
   `--join <link>`; the other hero is drawn and animated with the same
   clips as the player, from 30 snapshots a second. Measured with one
   alpha and `coop_peer` (a game-less stand-in, `m2-net/examples`), so
   the PC runs one alpha per test.
2. **Over the internet:** the same through the relay and from another
   network. The relay path is measured already; a run from a second
   machine needs a friend.
3. **Shared world:** the host's people, vehicles and the contract's state
   (objectives, markers) shown on the friend's side; the friend's hits
   and vehicle use sent to the host. Snapshots of what is near either
   player, events on the reliable stream.
4. **The beta package:** a release workflow that builds the i686 exe and
   zips it; the join page; the in-game host and join menu (the pause
   menu's co-op line: invite, copy the link, accept, leave); the folder
   picker.
5. **Co-op play:** both heroes in vehicles (driver and gunner), the
   contract's co-op rules, reviving a downed partner (invented where the
   data doesn't decide).

## Not decided yet (the host's call)

- Hosting the join page on GitHub Pages and publishing releases (both
  public, both free).
- Whether to run our own relay later.
