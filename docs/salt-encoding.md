# The SALT build-order encoding (v5)

SALT is the build-order string format used by SpawningTool and by the SALT practice mod
available in StarCraft II's custom-games section. This project stores build orders in SALT
because it is the format the existing ecosystem already speaks, so an order can be pasted
into the in-game mod or a SpawningTool page without translation.

This document is this project's specification. It was derived by decoding real SALT strings
from live SpawningTool pages and checking the result against each page's own build-order
table, and it corrects two errors in the circulated v5 text document - see
[Corrections](#corrections-to-the-circulated-v5-document).

## Anatomy of a SALT string

    $203761|spawningtool.com||~  ? ;* U 8,!, 3-!E 9-!N ...
    ^      ^                  ^^ ^
    |      |                  |  +-- payload: 5 characters per step, nothing else
    |      |                  +----- the tilde that closes the metadata
    |      +------------------------ author, then description (may be empty)
    +------------------------------- title, with a leading "$" sigil

A string is `metadata ~ payload`. Metadata is pipe-separated as
`title|author|description|`, and the whole metadata block **starts with `$`**. The `$` is a
format sigil, not part of the title: a SpawningTool title like `$b'ThikiPizza'-130000` keeps
only the single leading `$` and the rest is literal title text.

The payload is a flat sequence of **5-character steps with no separator**. There is no count,
no checksum, and no escaping - the length is always an exact multiple of five, and a payload
whose length is not divisible by five is malformed.

## The character set

Each character encodes one integer by its **index into printable ASCII**, from the space
character (`0x20`) upwards:

    index 0  -> space character
    index 7  -> single quote
    index 8  -> open parenthesis
    index 9  -> close parenthesis
    index 11 -> plus sign
    index 93 -> close brace

Formally the alphabet is code points `0x20` to `0x7E` in order, so **`value = ord(char) - 32`**
and **`char = chr(value + 32)`**. Only codes `0` to `93` occur; there is no base conversion and
no multi-character number. This is why a SALT string is full of punctuation - every step
spends five characters spelling small numbers in a dense alphabet.

## A step is five characters

| Position | Meaning | Encoding |
|---|---|---|
| 1 | supply | `value + 4`, or **space for "no supply"** |
| 2 | minutes | plain value (`0` to `59`+) |
| 3 | seconds | plain value (`0` to `59`) |
| 4 | item type | `0` structure, `1` unit, `2` morph, `3` upgrade |
| 5 | item code | row within that type's table |

### Supply: value + 4, not value + 10

The supply column is stored as an offset that makes the common early-game values small.
**Supply = character value + 4.** A space (value `0`) means the step carries no supply
information at all and the reader should ignore the column; blank supply is common for
`Action`-style steps and for builds where supply is not meaningful.

This was verified against 335 decoded steps across 10 live build pages: `value + 4` equals
the supply the page itself displays on **all 335**, with 327 exact and 8 differing by 1 to 4
where the page rounds or merges rows. `value + 10` disagrees with every one of them.

The circulated document's stated floor of 11 supply is a convention of the pages, not a
limit of the format: real strings encode minimums down to 10 (`value 6`), and nothing
prevents a lower value.

### Time is minutes and seconds, separately

Position 2 is whole minutes and position 3 is whole seconds; both are plain values with no
offset. Seconds therefore range `0` to `59` and minutes are unbounded in the format (observed
up to 11). Time is **in-game seconds** - the SC2 clock, which in Legacy of the Void runs 1:1
with real time at `Faster` speed.

Note that SpawningTool's own table displays times one second later than SALT does on most
rows (for example SALT says `0:31`, the table says `0:32`). This is a display convention, not
a unit difference; SALT's value is the one to store.

## Item tables

An item is a `(type, code)` pair. Codes are **not** contiguous or grouped by race - Terran,
Protoss and Zerg entries are interleaved within each type. Decode with these tables.

### Type 0 - Structures

| Code | Item | Code | Item | Code | Item |
|---|---|---|---|---|---|
| `00` | Armory | `01` | Barracks | `02` | Bunker |
| `03` | Command Center | `04` | Engineering Bay | `05` | Factory |
| `06` | Fusion Core | `07` | Ghost Academy | `08` | Missile Turret |
| `09` | Reactor (Barracks) | `10` | Reactor (Factory) | `11` | Reactor (Starport) |
| `12` | Refinery | `13` | Sensor Tower | `14` | Starport |
| `15` | Supply Depot | `16` | Tech Lab (Barracks) | `17` | Tech Lab (Factory) |
| `18` | Tech Lab (Starport) | `19` | Assimilator | `20` | Cybernetics Core |
| `21` | Dark Shrine | `22` | Fleet Beacon | `23` | Forge |
| `24` | Gateway | `25` | Nexus | `26` | Photon Canon |
| `27` | Pylon | `28` | Robotics Bay | `29` | Robotics Facility |
| `30` | Stargate | `31` | Templar Archives | `32` | Twilight Council |
| `33` | Baneling Nest | `46` | Creep Tumor | `34` | Evolution Chamber |
| `35` | Extractor | `36` | Hatchery | `37` | Hydralisk Den |
| `38` | Infestation Pit | `39` | Nydus Network | `40` | Roach Warren |
| `41` | Spawning Pool | `42` | Spine Crawler | `43` | Spire |
| `44` | Spore Crawler | `45` | Ultralisk Cavern |  |  |

### Type 1 - Units

| Code | Item | Code | Item | Code | Item |
|---|---|---|---|---|---|
| `00` | Banshee | `40` | Battle Hellion | `01` | Battlecruiser |
| `48` | Cyclone | `02` | Ghost | `03` | Hellion |
| `49` | Liberator | `04` | Marauder | `05` | Marine |
| `06` | Medivac | `07` | Raven | `08` | Reaper |
| `09` | SCV | `10` | Siege Tank | `11` | Thor |
| `41` | Warhound | `42` | Widow Mine | `12` | Viking |
| `51` | Adepts | `14` | Carrier | `15` | Colossus |
| `16` | Dark Templar | `50` | Disruptor | `17` | High Templar |
| `18` | Immortal | `19` | Mothership | `43` | Mothership Core |
| `20` | Observer | `44` | Oracle | `21` | Phoenix |
| `22` | Probe | `23` | Sentry | `24` | Stalker |
| `45` | Tempest | `25` | Void Ray | `39` | Warp Prism |
| `26` | Zealot | `27` | Corruptor | `28` | Drone |
| `29` | Hydralisk | `38` | Infestor | `30` | Mutalisk |
| `31` | Overlord | `32` | Queen | `33` | Roach |
| `46` | Swarm Host | `34` | Ultralisk | `47` | Viper |
| `35` | Zergling |  |  |  |  |

### Type 2 - Morphs

| Code | Item | Code | Item | Code | Item |
|---|---|---|---|---|---|
| `00` | Orbital Command | `01` | Planetary Fortress | `02` | Warp Gate |
| `13` | Archon | `03` | Lair | `04` | Hive |
| `05` | Greater Spire | `06` | Brood Lord | `07` | Baneling |
| `08` | Overseer | `09` | ravager | `10` | lurker |
| `12` | lurker den |  |  |  |  |

### Type 3 - Upgrades

| Code | Item | Code | Item | Code | Item |
|---|---|---|---|---|---|
| `00` | Terran Building Armor | `01` | Terran Infantry Armor | `02` | Terran Infantry Weapons |
| `03` | Terran Ship Plating | `04` | Terran Ship Weapons | `05` | Terran Vehicle Plating |
| `06` | Terran Vehicle Weapons | `07` | 250mm Strike Cannons | `08` | Banshee - Cloaking |
| `09` | Ghost - Cloaking | `10` | Hellion - Pre-igniter | `11` | Marine - Stimpack |
| `12` | Raven - Seeker Missiles | `13` | Siege Tank - Siege Tech | `46` | Ghost - Moebius Reactor |
| `14` | Bunker - Neosteel Frame | `15` | Marauder - Concussive Shells | `16` | Marine - Combat Shields |
| `17` | Reaper Speed | `51` | Battlecruiser - Behemoth Reactor | `52` | Battlecruiser - Weapon Refit |
| `53` | Hi-Sec Auto Tracking | `54` | Medivac - Caduceus Reactor | `55` | Raven - Corvid Reactor |
| `56` | Raven - Durable Materials | `57` | Hellion - Transformation servos | `66` | Drilling claws |
| `18` | Protoss Ground Armor | `19` | Protoss Ground Weapons | `20` | Protoss Air Armor |
| `21` | Protoss Air Weapons | `22` | Protoss Shields | `23` | Sentry - Hallucination |
| `24` | High Templar - Psi Storm | `25` | Stalker - Blink | `26` | Warp Gate Tech |
| `27` | Zealot - Charge | `47` | Extended Thermal Lance | `58` | Carrier - Graviton Catapult |
| `59` | Observer - Gravatic Boosters | `60` | Warp Prrism - Gravatic Drive | `61` | Oracle - Bosonic Core |
| `62` | Tempest - Gravity Sling | `67` | Anion Pulse-Crystals | `28` | Zerg Ground Carapace |
| `29` | Zerg Melee Weapons | `30` | Zerg Flyer Carapace | `31` | Zerg Flyer Weapons |
| `32` | Zerg Missile Weapons | `33` | Hydralisk - Grooved Spines | `34` | Overlord - Pneumatized Carapace |
| `35` | Overlord - Ventral Sacs | `36` | Roach - Glial Reconstitution | `38` | Roach - Tunneling Claws |
| `40` | Ultralisk - Chitinous Plating | `41` | Zergling - Adrenal Glands | `42` | Zergling - Metabolic Boost |
| `44` | Burrow | `45` | Centrifugal Hooks | `49` | Neural Parasite |
| `50` | Pathogen Gland | `64` | Swarm Host - Evolve Enduring Locusts | `65` | Hydralisk - Muscular Augments |
| `68` | flying locusts | `69` | seismic spines | `71` | targeting optics |
| `72` | advanced ballistics | `73` | Resonating Glaives |  |  |

## Lossiness is a property of the format

SALT intentionally cannot represent every build order, and SpawningTool says so on each build
page: *"due to size limitations, all workers are removed. Additionally, only the first 3
supply buildings are included, and only the first 10 of any other unit are included."*

All three claims were checked against 22 real payloads:

| Rule | Observed |
|---|---|
| Workers removed | **0** SCV/Probe/Drone steps across all 22 payloads |
| At most 3 supply buildings | no supply building exceeded 3 |
| At most 10 of any other unit | the most-repeated single unit was exactly 10 (Dark Templar) |

Two further losses follow from the same table-driven design rather than from a deliberate cap:

- **No supply for most steps.** Only `0` to `93` supply values are encodable, and the space
  marker is used freely, so any supply a producer did not record is absent.
- **No `Action` steps.** SpawningTool's build pages contain rows whose name is `Action` with
  the real instruction in the note column ("Hallucinate Phoenix, send to Terran main",
  "Proxy Pylon"). `Action` has no `(type, code)` entry, so SALT cannot carry it. A decoded
  SALT string will therefore contain fewer steps than the page it came from, and the missing
  ones are exactly the ones with no unit or building to name.

A reader must not treat SALT as a complete record of a build order, and an encoder must not
pretend it can round-trip an arbitrary build. The safe contract is: **encode only steps whose
name exists in these tables, drop the rest, and say how many were dropped.**

## Corrections to the circulated v5 document

The `.txt` version of this scheme contains two errors, both settled by decoding real strings:

1. **Supply offset.** It states supply is `letter position + 10` and that the minimum supply
   for a build order is 11. In reality the offset is **+4**, and real payloads encode supply
   as low as 10.
2. **Type numbering in prose.** It labels the fourth character's value `3` as upgrades while
   its own tables put morphs at type 2 and upgrades at type 3. The tables are right; real
   payloads encode Warp Gate, Lair, Hive, Baneling and Archon as type `2` and upgrades such
   as Metabolic Boost as type `3`.

The tables themselves are accurate and complete - a decode/re-encode round trip over 13 live
build pages reproduced the original strings byte for byte.

## Reference algorithm

```text
DECODE(salt):
    title, rest = split_once(salt, "~")
    assert rest.length % 5 == 0
    for i in 0, 5, 10, ... len(rest):
        c = rest[i .. i+5]
        supply  = ord(c[0]) - 32        # 0 means absent
        minutes = ord(c[1]) - 32
        seconds = ord(c[2]) - 32
        type    = ord(c[3]) - 32
        code    = ord(c[4]) - 32
        yield (supply+4 if supply else None, minutes, seconds,
               TABLES[type][code])

ENCODE(steps):
    out = ""
    for (supply, minutes, seconds, name) in steps:
        type, code = REVERSE[name]      # skip the step if the name is absent
        out += chr((supply-4 if supply else 0) + 32)
        out += chr(minutes + 32)
        out += chr(seconds + 32)
        out += chr(type + 32)
        out += chr(code + 32)
    return out
```

Guard both directions: raise on a payload length that is not a multiple of five, on a
character outside `0x20` to `0x7E`, on an item code absent from the table, and on a value that
would fall outside `0` to `93`. Silently substituting a code produces a build order that looks
valid and is not.
