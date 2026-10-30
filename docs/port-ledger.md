# Port ledger

`tools/port_ledger.py` lists every vanilla method next to the Horizon functions that port it, both ways, and
points at the places where a port exists but isn't wired like vanilla. It answers the questions every parity
fix starts with, without searching:

- where is vanilla's `Class.method` in Horizon (`where`), and which vanilla method does this Rust function
  port (`what`);
- which vanilla code and which Rust functions a bug report is about (`issue`);
- what a branch touches, and which gaps it opens or closes (`check`).

`vanilla_map.py` maps classes to files for version ports; the ledger works one level down, per method and per
Rust function with its line span.

## Use

```sh
python3 tools/port_ledger.py build                                   # about 20 s; again after pulling
python3 tools/port_ledger.py where KineticWeapon.damageEntities       # its ports, callees, callers, gaps
python3 tools/port_ledger.py where ShulkerBulletRenderer              # every method of a class, with status
python3 tools/port_ledger.py what crates/horizon-client/src/game/spear.rs:150
python3 tools/port_ledger.py class IronGolem                          # method by method: where, or missing
python3 tools/port_ledger.py issue 3123                               # a GitHub issue (or the report's text)
python3 tools/port_ledger.py check --base origin/main                 # what this branch touches
python3 tools/port_ledger.py coverage --package world/entity --depth 3
python3 tools/port_ledger.py gaps --kind seam                         # see the kinds below
python3 tools/port_ledger.py report                                   # .local/port-ledger/report.{md,html}
python3 tools/port_ledger.py diff old/ledger.json                     # against another build
python3 tools/port_ledger.py todo --package world/entity/animal       # the next methods with no answer
```

`--json` on any query gives machine output for agents.

`build` reads the vanilla game jar (the lab's, or the launcher's `versions/<version>/<version>.jar`; `--jar`
picks one, `--no-jar` skips it) and the lab's decompiled sources (`.local/minecraft/<version>/sources/
{client,server}`, from `python3 tools/lab.py decompile`; `--sources DIR` takes any directory of decompiled
Java). The jar's bytecode gives the exact call targets, constants and ids; the sources give line numbers and
the cases the bytecode can't name. Nothing from either is written outside `.local/`.

`report` writes `report.md` and `report.html`, a single offline page with a search over every method (by class,
method, Rust function or file), the gaps by kind, and coverage by package.

## How a method is matched

1. **Ported:** a Rust function whose doc comment opens with the method, the house style
   (`` /// `KineticWeapon.damageEntities`: one tick of the spear in use. ``). `Class.method`, `Class::method` and
   `Class#method` all count, as in `vanilla_map.py`; `Outer.Inner.method` names a nested class. A method a
   class inherits resolves to the class that declares it.
2. **Named:** a function with the method's own name (`tickRain` → `tick_rain`) in a file that cites the class,
   in an `impl` of a type named for the class (`impl ShulkerBullet`), or in a file named for it
   (`shulker_bullet.rs`). A function in `impl HurtByTarget` is never taken for `AvoidEntityGoal.canUse`: its
   type must be named for the class, a sub- or superclass of it, or be one of Horizon's own containers
   (`Game`, `Sim`, `MotionWorld`).
3. **Mapped, skipped, absent:** a row in the verdict map (below) says where it is, why it needs no port, or
   that Horizon doesn't do it.
4. **Mentioned:** the method appears in some other comment of a function (often a caller explaining a step).
5. **Linked:** where `X` is ported as `A`, a callee of `X` with no port and a callee of `A` nothing cites,
   whose names agree less Java's `get` (`getFlyingSpeed` → `flying_speed`).
6. **Structural:** in the class's home files (below), the function that touches the same state (the fields the
   method reads and writes, by their Rust names) and reaches the ports of the same callees, weighed against its
   length so one long function doesn't take every method.
7. **Inferred:** nothing names it, but a function shares what is hard to share by chance: rare numbers
   (`0.458`, `72000`) and game ids (`SoundEvents.ENTITY_SHULKER_BULLET_HIT` against
   `"entity.shulker_bullet.hit"`, `Blocks.DETECTOR_RAIL` against `"minecraft:detector_rail"`).
8. **Trivial:** an accessor, an abstract method, a few instructions, or a constructor that only sets fields
   (the struct's initialiser): nothing to port on its own. Coverage leaves these out of its percentage.
9. **Missing:** none of these: no answer yet.

Every class also gets a **home**: the Rust files it lives in, weighed from all of the above, the files citing
it, files named for it and `impl`s of a type named for it (a nested class without one lives with its outer
class). `where Class`, `class` and `issue` show it.

`coverage` gives two shares: *found*, the methods worth porting (not trivial, not skipped) that have a port,
and *answered*, all methods with any answer (a port, a reason to skip, trivial, or known to be missing).

Citing the vanilla method in the first line of a function's doc is what makes it findable, by people, agents
and this tool alike; `check` lists the functions a branch changed that look like ports but cite nothing. Test
code (`#[test]`, `mod tests`, fixtures, probes, examples) never counts as a port.

## Gaps

Java calls come from the bytecode (`invokevirtual`, `invokestatic`, method handles in lambdas), so each names
its exact owner; a call also reaches the overrides in subclasses. Rust calls resolve through the receiver's
type where it is known (`self`, a parameter, a `let`, a struct field, a trait to its impls), else by name.

| Kind | Means | Example it found |
|---|---|---|
| `seam` | A vanilla method ported on two paths (the in-frame game, the server-client side in `game/remote*.rs`, the server in `game/lan*.rs` and `src/lan/`), where one path reaches the port of a callee and the other doesn't. Single player runs on the server thread, so this is where most player-visible bugs sit; a gap on the old in-frame path sorts last. | `remote_use` ported `Minecraft.startUseItem` without `LocalPlayer.itemUsed`: the held block never dipped after a placement on a server |
| `client` | A vanilla method with an effect only the client's copy of the world shows (`addParticle`, `playLocalSound`, `itemUsed`, an `isClientSide()` branch), whose port nothing in `game/remote*.rs` reaches. | `AbstractHorse.handleEating`'s happy-villager particle, drawn only on the server's level |
| `drift` | A port that never names a sound, particle, item, block, effect or game event its vanilla method uses, or lacks one of its rare numbers. Names count however Rust spells them (`Sound::AnvilLand`, `"block.anvil.land"`). | `throw_ender_eye`, the port of `EnderEyeItem.use`, names neither the `used_ender_eye` trigger nor the `projectile_shoot` game event (a lead, not yet checked) |
| `class` | A whole class nothing ports while most of its package is ported. Renderers, sound instances and goals are reached by registration, so no call graph shows them. | `ShulkerBulletRenderer`, `ElytraOnPlayerSoundInstance` |
| `callee` | A ported method whose ported callee its port never reaches. Noisier: a port may get the value as an argument instead. | `Projectile.shootFromRotation` without `Entity.getKnownMovement`: throws added a stale server velocity; `ServerPlayer.getKnownMovement` without `getVehicle`: a spear's charge ignored the horse |
| `stale` | A citation whose class or method the pinned version doesn't have (renamed, removed, or a typo). | |
| `map` | A verdict-map row whose class, method or Rust fn doesn't exist, or whose verdict isn't one of the above. | |
| `unported` | (`gaps --kind unported`) Vanilla methods with no port, ranked by how many vanilla methods call them. | |

Calls to maths, geometry, randomness and text (`Mth`, `AABB`, `Vec3`, `RandomSource`, `Component`) never make
a `seam` or `callee` gap: every path does those its own way.

A gap is a lead, not a verdict: read the vanilla method and the Rust function it names before changing
anything.

## The verdict map

What no automatic match finds, someone reads and decides. `tools/port_map/*.tsv` holds those answers, one
file per batch, tab-separated:

```
Class.method	verdict	where	note
XoroshiroRandomSource.nextInt	port	crates/horizon-core/src/random.rs::XoroshiroRandom::next_int_bound	bounded form
ArmadilloAi.initIdleActivity	inline	crates/horizon-client/src/mobs/armadillo.rs::register	gap: no AnimalMakeLove, so armadillos never breed
MushroomCow.addAdditionalSaveData	missing		Type is not saved: a mooshroom turned brown by lightning loads red
NoiseRouterData	skip:data		a class row covers the class's methods without a row of their own
```

Verdicts: `port` (the fn that does the method's job), `inline` (the bigger fn its behaviour is folded into),
`missing`, `skip:<reason>` with a reason from `java`, `data`, `ui`, `debug`, `service`, `server`, `other`, and on
a class row `home` with the class's files. `where` names a fn by file, impl type and name, not by line, so rows
survive edits around them; a row whose class, method or fn doesn't exist shows up as a `map` gap.

`todo [--package P] [--limit N] [--after CLASS]` prints the next batch of methods with no answer, whole classes
at a time, with each class's Java source, home files, and Rust functions in them with similar names: the input
for whoever (or whichever agent) writes the next batch file. Set `PORT_LEDGER_OUT` to build into another
directory, so several people can check their own rows at once.

## Parity harness coverage

A mapped method says where the port is, not that it behaves like vanilla; the parity harnesses check that
(`tools/test_*_parity.py`: a vanilla oracle in `mods/horizon/src/test/java/dev/horizonlab/parity/` and a Rust
fixture or probe under `crates/*/examples/` run on the same inputs). The ledger links them: a harness covers a
ported method when its fixture reaches the port and the vanilla side is in reach (the oracle calls into the
method, or the harness or oracle names its class). `where` lists a method's harnesses, and
`gaps --kind untested [--package world/entity]` lists the sure pairs (cited or mapped) with behaviour of their
own that no harness covers, most-called first: where a new harness would catch the most drift.

## Bug reports and branches

`issue N` reads a GitHub issue (`gh issue view`), or takes the report's text (`-` reads stdin), drops the
bug-report template, and ranks the vanilla classes it names the way players write them ("shulker bullet",
"eye of ender", and registry names from the entity, block and item registries: "mooshroom" is `MushroomCow`), then the methods its words name in those classes. For each class it lists where Horizon ports
it and the gaps around it: on #3123 ("an iron golem you built no longer attacks you") it points at
`IronGolem.isPlayerCreated` and `setPlayerCreated`, both missing.

`check --base origin/main` takes the branch's changes since its merge base (uncommitted work too), rebuilds the
ledger if it is older than them, and lists the ports the changes touch with their vanilla source and open gaps,
citations to add, and stale ones. It then builds the base in a throwaway worktree and lists the gaps the
branch opens and closes and the methods it newly ports (`--no-compare` skips that half, about 20 s).
