//! Deterministic simulation of the cell protocols on the real state
//! machine: three cells and a directory, each a `CellState` (Raft below it
//! is the other simulation's job, `raft_simulation.rs`), with executors
//! claiming and writing, clients sending input, a rebalancer moving
//! sessions, and move drivers that run the node's own move rules
//! (`everruns_cell::moves`). Every command and reply travels through one
//! network that delays, reorders, duplicates and drops; executors and move
//! drivers die mid-step and come back; proposers' clocks disagree by up to
//! a lease, so an expired epoch can look current again.
//!
//! Everything comes from the seed and nothing reads the wall clock or
//! spawns a task, so a seed replays exactly: each seed runs twice and the
//! two runs must end in the same state. A failing seed is a regression
//! test as it stands.
//!
//! What must hold at every step (the `Moves`, `Epochs` and `Grants`
//! models' invariants, on the code):
//! - one owner: at most one cell holds a session live, and that copy is
//!   the one the directory names (cell and generation);
//! - one writer: the writes a session accepts come from epochs that only
//!   grow, across every cell it lives in;
//! - a cell holds a reservation only for a session it holds, and no grant
//!   has more reserved and spent than it was granted.
//!
//! And once the faults stop and every move settles: every session lives in
//! exactly one cell, and every write an executor saw acknowledged is in it.
//!
//! It found three move bugs the kill runs had not (each now has a rule in
//! `state.rs` saying so): a late freeze retargeting a committed move, a
//! late thaw of an earlier freeze, and a write from an epoch that looked
//! expired to the driver landing after the copy was taken. It also found
//! the leaving copy's open reservation never closing.
//!
//! `SIM_TRACE=<part of a session id>` prints every command naming it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::{BTreeMap, BTreeSet};

use everruns::SessionId;
use everruns::hosting::SessionBinding;
use everruns::hosting::spi::session_commands::SessionCommand;
use everruns_cell::allowance::Grant;
use everruns_cell::authority::WriteOp;
use everruns_cell::moves;
use everruns_cell::state::{CellId, CellState, Command, Placed, Placement, Response};
use everruns_contracts::runtime::message::RuntimeMessage;
use everruns_contracts::typed_id::{EventId, MessageId};
use serde_json::{Value, json};

/// xorshift64*: everything random comes from here.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

const CELLS: [CellId; 3] = [1, 2, 3];
const DIRECTORY: CellId = 0;
const SESSIONS: usize = 6;
const EXECUTORS: usize = 4;
const TTL_MS: i64 = 1_000;
/// How long a move driver waits for a reply before giving up (the node's
/// gateway timeout); a new driver picks the move up again.
const DRIVER_TIMEOUT_MS: i64 = 3_000;
/// A prepared copy waits this long before asking the directory (the
/// node's `PREPARED_GRACE`).
const PREPARED_GRACE_MS: i64 = 2_000;
const TENANT: &str = "t";
const PER_CALL: i64 = 100;

/// A session id in an ordered map (ids don't order; their uuids do).
type Key = uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Driver {
    /// Moving `session` out of `cell`.
    Move,
    /// Settling a prepared copy in `cell`.
    Prepared,
}

#[derive(Clone, Debug)]
enum Reply {
    None,
    /// To executor `n`'s request `life` (a new one per request, so a
    /// duplicated reply to an earlier request is not taken for this one).
    Executor {
        n: usize,
        life: u64,
    },
    /// To the driver of (`cell`, `session`), for its step `life`.
    Driver {
        cell: CellId,
        session: SessionId,
        kind: Driver,
        life: u64,
    },
}

#[derive(Clone, Debug)]
struct Msg {
    to: CellId,
    command: Command,
    reply: Reply,
}

#[derive(Clone, Debug)]
enum Step {
    /// Waiting on the revoke of the leaving copy's epoch, to move to `to`.
    Revoke { to: CellId },
    /// Waiting on the import of the copy at `leaving + 1` into `to`.
    Import { to: CellId, leaving: u64 },
    /// Waiting on the directory to place `proposed` over `leaving`.
    Place { leaving: u64, proposed: Placed },
    /// Sent the first of `rest`; waiting on its reply.
    Settle { rest: Vec<(CellId, Command)> },
    /// Waiting on the directory's entry for a prepared copy at `generation`.
    Locate { generation: u64 },
    /// Sent the last command; waiting on its reply to finish.
    Last,
}

struct Running {
    life: u64,
    step: Step,
    deadline: i64,
}

#[derive(Default)]
struct Executor {
    life: u64,
    /// The epoch it believes it holds: cell, session, fence.
    holding: Option<(CellId, SessionId, u64)>,
    waiting: bool,
    written: u64,
}

struct World {
    rng: Rng,
    now: i64,
    cells: BTreeMap<CellId, CellState>,
    net: Vec<Msg>,
    executors: Vec<Executor>,
    drivers: BTreeMap<(CellId, Key, Driver), Running>,
    /// When each prepared copy was first seen, per cell.
    prepared_since: BTreeMap<(CellId, Key), i64>,
    sessions: Vec<SessionId>,
    /// Commit keys an executor saw acknowledged, per session.
    acked: BTreeMap<Key, BTreeSet<String>>,
    /// Highest fence a session accepted a write from, anywhere.
    fences: BTreeMap<Key, u64>,
    lives: u64,
    ids: u128,
    faults: bool,
    stats: BTreeMap<&'static str, u64>,
}

impl World {
    fn new(seed: u64) -> Self {
        let mut world = Self {
            rng: Rng::new(seed),
            now: 1_000_000,
            cells: BTreeMap::new(),
            net: Vec::new(),
            executors: (0..EXECUTORS).map(|_| Executor::default()).collect(),
            drivers: BTreeMap::new(),
            prepared_since: BTreeMap::new(),
            sessions: Vec::new(),
            acked: BTreeMap::new(),
            fences: BTreeMap::new(),
            lives: 0,
            ids: u128::from(seed) << 64,
            faults: true,
            stats: BTreeMap::new(),
        };
        for cell in std::iter::once(DIRECTORY).chain(CELLS) {
            world.cells.insert(cell, CellState::default());
        }
        for cell in CELLS {
            world.apply(
                cell,
                Command::Grant {
                    tenant: TENANT.into(),
                    grant_id: format!("g{cell}"),
                    grant: Grant {
                        granted_micros: 20 * PER_CALL,
                        per_call_micros: PER_CALL,
                        spend_until_ms: i64::MAX,
                        used_micros: 0,
                        overrun_micros: 0,
                    },
                },
            );
        }
        // Placed first, then created where placed, as the gateway does.
        let definition = json!({ "name": "sim" });
        let build =
            everruns_cell::hosting::build_for(&definition, everruns_cell::hosting::BUILD_ID);
        let binding = serde_json::to_value(SessionBinding {
            framework: 1,
            behavior: build,
            harness: None,
            environment: None,
            metadata: Value::Null,
        })
        .unwrap();
        for _ in 0..SESSIONS {
            let session = SessionId::from_uuid(world.uuid());
            let cell = CELLS[world.rng.below(CELLS.len() as u64) as usize];
            let placed = world.apply(
                DIRECTORY,
                Command::Place {
                    session,
                    cell,
                    generation: 1,
                    previous: 0,
                },
            );
            assert!(matches!(placed, Response::Ok), "{placed:?}");
            let seed = EventId::from_uuid(world.uuid());
            let created = world.apply(
                cell,
                Command::Host {
                    session,
                    command: SessionCommand::Create {
                        binding: binding.clone(),
                    },
                    definition: Some(definition.clone()),
                    now_ms: world.now,
                    seed,
                },
            );
            assert!(matches!(created, Response::Value { .. }), "{created:?}");
            let assigned = world.apply(
                cell,
                Command::Assign {
                    session,
                    tenant: TENANT.into(),
                },
            );
            assert!(matches!(assigned, Response::Ok), "{assigned:?}");
            world.sessions.push(session);
        }
        world
    }

    fn uuid(&mut self) -> uuid::Uuid {
        self.ids += 1;
        uuid::Uuid::from_u128(self.ids)
    }

    fn count(&mut self, what: &'static str) {
        *self.stats.entry(what).or_default() += 1;
    }

    /// The time a proposer stamps on a command: its clock may be behind by
    /// up to a lease (a new leader's), never ahead.
    fn stamp(&mut self) -> i64 {
        if self.faults {
            self.now - self.rng.below(TTL_MS as u64) as i64
        } else {
            self.now
        }
    }

    fn send(&mut self, to: CellId, command: Command, reply: Reply) {
        self.net.push(Msg { to, command, reply });
    }

    /// Apply `command` in `cell` and check every invariant after it.
    fn apply(&mut self, cell: CellId, command: Command) -> Response {
        let write = match &command {
            Command::Write { session, fence, .. } => Some((*session, *fence)),
            _ => None,
        };
        let trace = std::env::var("SIM_TRACE").ok().filter(|t| {
            let text = format!("{command:?}");
            text.contains(t.as_str())
        });
        let summary = trace.as_ref().map(|_| {
            let text = format!("{command:?}");
            text.chars().take(160).collect::<String>()
        });
        let state = self.cells.get_mut(&cell).unwrap();
        let response = state.apply(command, &mut Vec::new());
        if let Some(summary) = summary {
            let r = format!("{response:?}");
            eprintln!(
                "t{} cell {cell}: {summary} -> {}",
                self.now,
                &r[..r.len().min(120)]
            );
        }
        if let (Some((session, fence)), Response::Value { .. }) = (write, &response) {
            let last = self.fences.entry(session.uuid()).or_default();
            assert!(
                fence >= *last,
                "one writer: {session} took a write from epoch {fence} after {last}"
            );
            *last = fence;
        }
        self.check();
        response
    }

    fn check(&self) {
        let directory = &self.cells[&DIRECTORY].directory;
        for session in &self.sessions {
            let live: Vec<(CellId, u64)> = CELLS
                .iter()
                .filter_map(|cell| {
                    self.cells[cell]
                        .sessions
                        .get(session)
                        .filter(|s| s.placement == Placement::Live)
                        .map(|s| (*cell, s.generation))
                })
                .collect();
            assert!(live.len() <= 1, "one owner: {session} is live in {live:?}");
            if let Some((cell, generation)) = live.first() {
                assert_eq!(
                    directory.get(session),
                    Some(&Placed {
                        cell: *cell,
                        generation: *generation
                    }),
                    "the live copy of {session} is the one the directory names"
                );
            }
        }
        for cell in CELLS {
            let state = &self.cells[&cell];
            for (tenant, allowance) in &state.allowances.tenants {
                for session in allowance.reserved.keys() {
                    assert!(
                        state.sessions.contains_key(session),
                        "cell {cell} holds a reservation of {tenant} for {session}, which left"
                    );
                }
                for (id, grant) in &allowance.grants {
                    let held: i64 = allowance
                        .reserved
                        .values()
                        .filter(|r| &r.grant == id)
                        .map(|r| r.micros)
                        .sum();
                    assert!(
                        grant.used_micros + held <= grant.granted_micros + grant.overrun_micros,
                        "cell {cell} grant {id}: {} used + {held} held over {} granted",
                        grant.used_micros,
                        grant.granted_micros
                    );
                }
            }
        }
    }

    /// Where a caller would send work for `session`: the directory's cell.
    fn home(&self, session: SessionId) -> Option<CellId> {
        self.cells[&DIRECTORY]
            .directory
            .get(&session)
            .map(|p| p.cell)
    }

    fn pick_session(&mut self) -> SessionId {
        self.sessions[self.rng.below(self.sessions.len() as u64) as usize]
    }

    /// One step of the world: something acts, or a message arrives.
    fn step(&mut self) {
        self.now += 1 + self.rng.below(40) as i64;
        self.spawn_drivers();
        match self.rng.below(100) {
            0..=44 => self.deliver(),
            45..=59 => self.executor_acts(),
            60..=66 => self.client_sends(),
            67..=69 => self.rebalance(),
            70..=72 if self.faults => self.crash_executor(),
            73..=74 if self.faults => self.crash_driver(),
            _ => self.time_out_drivers(),
        }
    }

    fn deliver(&mut self) {
        if self.net.is_empty() {
            return;
        }
        let i = self.rng.below(self.net.len() as u64) as usize;
        let msg = if self.faults && self.rng.chance(10) {
            self.count("duplicated");
            self.net[i].clone()
        } else {
            self.net.swap_remove(i)
        };
        if self.faults && self.rng.chance(5) {
            self.count("dropped");
            return;
        }
        let response = self.apply(msg.to, msg.command.clone());
        if self.faults && self.rng.chance(5) {
            self.count("reply lost");
            return;
        }
        match msg.reply {
            Reply::None => {}
            Reply::Executor { n, life } => self.executor_reply(n, life, &msg.command, response),
            Reply::Driver {
                cell,
                session,
                kind,
                life,
            } => self.driver_reply(cell, session, kind, life, response),
        }
    }

    fn client_sends(&mut self) {
        let session = self.pick_session();
        let Some(cell) = self.home(session) else {
            return;
        };
        let mut message = RuntimeMessage::user("go");
        message.id = MessageId::from_uuid(self.uuid());
        message.created_at = chrono::DateTime::from_timestamp_millis(self.now).unwrap_or_default();
        let seed = EventId::from_uuid(self.uuid());
        let now_ms = self.stamp();
        self.send(
            cell,
            Command::Host {
                session,
                command: SessionCommand::Admit {
                    message: Box::new(message),
                    cancelled: false,
                    context: None,
                },
                definition: None,
                now_ms,
                seed,
            },
            Reply::None,
        );
    }

    fn executor_acts(&mut self) {
        let n = self.rng.below(EXECUTORS as u64) as usize;
        if self.executors[n].waiting {
            return;
        }
        self.lives += 1;
        let life = self.lives;
        self.executors[n].life = life;
        let holder = format!("e{n}");
        let now_ms = self.stamp();
        let reply = Reply::Executor { n, life };
        let (cell, command) = match self.executors[n].holding {
            None => {
                let session = self.pick_session();
                let Some(cell) = self.home(session) else {
                    return;
                };
                (
                    cell,
                    Command::Claim {
                        session,
                        holder,
                        node: 1,
                        ttl_ms: TTL_MS,
                        now_ms,
                    },
                )
            }
            Some((cell, session, fence)) => match self.rng.below(10) {
                0..=4 => {
                    let written = self.executors[n].written;
                    (
                        cell,
                        Command::Write {
                            session,
                            fence,
                            change: WriteOp::Commit {
                                key: format!("step/{fence}/{holder}/{written}"),
                                value: json!(written),
                                absent: false,
                            },
                            now_ms,
                        },
                    )
                }
                5..=6 => (cell, Command::Reserve { session, now_ms }),
                7..=8 => (
                    cell,
                    Command::Renew {
                        session,
                        holder,
                        fence,
                        ttl_ms: TTL_MS,
                        consumed: 0,
                        now_ms,
                    },
                ),
                _ => {
                    let hand_off = self.rng.chance(50);
                    (
                        cell,
                        Command::Release {
                            session,
                            holder,
                            fence,
                            consumed: 0,
                            hand_off,
                            now_ms,
                        },
                    )
                }
            },
        };
        self.executors[n].waiting = true;
        self.send(cell, command, reply);
    }

    fn executor_reply(&mut self, n: usize, life: u64, command: &Command, response: Response) {
        if self.executors[n].life != life {
            return; // the incarnation that asked is gone
        }
        let executor = &mut self.executors[n];
        executor.waiting = false;
        match (command, response) {
            (
                Command::Claim {
                    session, holder, ..
                },
                Response::Value { value },
            ) => {
                if let Some(fence) = value["fence"].as_u64() {
                    let cell = self.home(*session).unwrap_or(DIRECTORY);
                    // The claim went to the cell that answered; the
                    // executor keeps writing there.
                    let cell = CELLS
                        .into_iter()
                        .find(|c| {
                            self.cells[c].sessions.get(session).is_some_and(|s| {
                                s.authority
                                    .holder
                                    .as_ref()
                                    .is_some_and(|h| h.name == *holder && h.fence == fence)
                            })
                        })
                        .unwrap_or(cell);
                    let executor = &mut self.executors[n];
                    executor.holding = Some((cell, *session, fence));
                    executor.written = 0;
                    self.count("claimed");
                }
            }
            (
                Command::Write {
                    session,
                    change: WriteOp::Commit { key, .. },
                    ..
                },
                Response::Value {
                    value: Value::Bool(true),
                },
            ) => {
                executor.written += 1;
                self.acked
                    .entry(session.uuid())
                    .or_default()
                    .insert(key.clone());
                self.count("acked writes");
            }
            (Command::Write { .. }, _) => executor.holding = None,
            (Command::Release { .. }, _) => {
                executor.holding = None;
                self.count("released");
            }
            (Command::Renew { .. }, Response::Value { value }) if value.is_null() => {
                executor.holding = None;
            }
            (Command::Renew { .. } | Command::Reserve { .. }, Response::Value { .. }) => {}
            (Command::Renew { .. } | Command::Reserve { .. }, _) => executor.holding = None,
            _ => {}
        }
    }

    fn crash_executor(&mut self) {
        let n = self.rng.below(EXECUTORS as u64) as usize;
        self.lives += 1;
        self.executors[n] = Executor {
            life: self.lives,
            ..Executor::default()
        };
        self.count("executor deaths");
    }

    fn rebalance(&mut self) {
        let session = self.pick_session();
        let Some(cell) = self.home(session) else {
            return;
        };
        let to = CELLS[self.rng.below(CELLS.len() as u64) as usize];
        self.send(
            cell,
            Command::Freeze {
                session,
                to: Some(to),
            },
            Reply::None,
        );
        self.count("moves asked");
    }

    /// What the node's tick does: a driver for every frozen session whose
    /// epoch is gone, and for every prepared copy past its grace.
    fn spawn_drivers(&mut self) {
        let mut wanted = Vec::new();
        for cell in CELLS {
            for (session, s) in &self.cells[&cell].sessions {
                match s.placement {
                    Placement::Frozen
                        if s.moving_to.is_some() && !s.authority.executing(self.now) =>
                    {
                        wanted.push((cell, *session, Driver::Move));
                    }
                    Placement::Prepared => wanted.push((cell, *session, Driver::Prepared)),
                    _ => {}
                }
            }
        }
        // Session maps hash in a per-process order; the world must not.
        wanted.sort_by_key(|(cell, session, kind)| (*cell, session.uuid(), *kind));
        self.prepared_since.retain(|key, _| {
            wanted
                .iter()
                .any(|(c, s, k)| (*c, s.uuid()) == *key && *k == Driver::Prepared)
        });
        for (cell, session, kind) in wanted {
            if self.drivers.contains_key(&(cell, session.uuid(), kind)) {
                continue;
            }
            if kind == Driver::Prepared {
                let since = *self
                    .prepared_since
                    .entry((cell, session.uuid()))
                    .or_insert(self.now);
                if self.now - since < PREPARED_GRACE_MS {
                    continue;
                }
            }
            self.start_driver(cell, session, kind);
        }
    }

    fn start_driver(&mut self, cell: CellId, session: SessionId, kind: Driver) {
        self.lives += 1;
        let life = self.lives;
        let reply = Reply::Driver {
            cell,
            session,
            kind,
            life,
        };
        let s = &self.cells[&cell].sessions[&session];
        let step = match kind {
            Driver::Prepared => {
                let generation = s.generation;
                self.send(DIRECTORY, Command::Locate { session }, reply);
                Step::Locate { generation }
            }
            Driver::Move => {
                let to = s.moving_to.unwrap_or(cell);
                if to == cell {
                    self.send(cell, moves::unfreeze(cell, session), reply);
                    Step::Last
                } else {
                    let now_ms = self.stamp();
                    self.send(cell, Command::Revoke { session, now_ms }, reply);
                    Step::Revoke { to }
                }
            }
        };
        self.drivers.insert(
            (cell, session.uuid(), kind),
            Running {
                life,
                step,
                deadline: self.now + DRIVER_TIMEOUT_MS,
            },
        );
    }

    fn driver_reply(
        &mut self,
        cell: CellId,
        session: SessionId,
        kind: Driver,
        life: u64,
        response: Response,
    ) {
        let key = (cell, session.uuid(), kind);
        let Some(running) = self.drivers.get(&key) else {
            return;
        };
        if running.life != life {
            return;
        }
        let Some(running) = self.drivers.remove(&key) else {
            return;
        };
        self.lives += 1;
        let life = self.lives;
        let reply = Reply::Driver {
            cell,
            session,
            kind,
            life,
        };
        let next = match running.step {
            Step::Revoke { to } => {
                let Response::Ok = response else {
                    return;
                };
                // Read right after the revoke applied, as the node reads
                // its replica once the proposal returns.
                let Some(s) = self.cells[&cell].sessions.get(&session) else {
                    return;
                };
                let mut image = everruns_cell::sm::move_image(s);
                let leaving = image.generation;
                image.generation = leaving + 1;
                self.send(
                    to,
                    Command::Import {
                        session,
                        image: Box::new(image),
                    },
                    reply,
                );
                Step::Import { to, leaving }
            }
            Step::Import { to, leaving } => {
                let Ok(proposed) = moves::after_import(cell, to, leaving, &response) else {
                    return; // failed: a new driver tries again
                };
                if proposed.cell == cell {
                    self.count("moves abandoned");
                }
                self.send(
                    DIRECTORY,
                    Command::Place {
                        session,
                        cell: proposed.cell,
                        generation: proposed.generation,
                        previous: leaving,
                    },
                    reply,
                );
                Step::Place { leaving, proposed }
            }
            Step::Place { leaving, proposed } => {
                let Ok(current) = moves::after_place(leaving, proposed, &response) else {
                    return;
                };
                let rest = moves::settle(cell, session, leaving, current);
                self.next_settle(rest, reply)
            }
            Step::Settle { rest } => {
                if !moves::step_done(&response) {
                    return;
                }
                self.next_settle(rest, reply)
            }
            Step::Locate { generation } => {
                let Response::Placement { current } = response else {
                    return;
                };
                let Some(command) = moves::settle_prepared(cell, session, generation, current)
                else {
                    return;
                };
                self.send(cell, command, reply);
                Step::Last
            }
            Step::Last => {
                if kind == Driver::Move && moves::step_done(&response) {
                    self.count("moves finished");
                }
                return;
            }
        };
        self.drivers.insert(
            key,
            Running {
                life,
                step: next,
                deadline: self.now + DRIVER_TIMEOUT_MS,
            },
        );
    }

    fn next_settle(&mut self, mut rest: Vec<(CellId, Command)>, reply: Reply) -> Step {
        let (to, command) = rest.remove(0);
        self.send(to, command, reply);
        if rest.is_empty() {
            Step::Last
        } else {
            Step::Settle { rest }
        }
    }

    /// The node behind a driver died: its next leader starts the step over.
    fn crash_driver(&mut self) {
        if self.drivers.is_empty() {
            return;
        }
        let i = self.rng.below(self.drivers.len() as u64) as usize;
        let key = *self.drivers.keys().nth(i).unwrap();
        self.drivers.remove(&key);
        self.count("driver deaths");
    }

    fn time_out_drivers(&mut self) {
        let now = self.now;
        self.drivers.retain(|_, running| running.deadline > now);
    }

    /// Faults off: deliver everything, let every move settle and every
    /// epoch expire.
    fn quiesce(&mut self) {
        self.faults = false;
        for executor in &mut self.executors {
            executor.holding = None;
            executor.waiting = false;
        }
        for _ in 0..200 {
            self.now += TTL_MS;
            self.time_out_drivers();
            self.spawn_drivers();
            while !self.net.is_empty() {
                self.deliver();
            }
            let unsettled = CELLS.iter().any(|cell| {
                self.cells[cell]
                    .sessions
                    .values()
                    .any(|s| s.placement != Placement::Live)
            });
            if !unsettled && self.drivers.is_empty() {
                return;
            }
        }
        panic!("moves never settled");
    }

    fn final_checks(&self, seed: u64) {
        for session in &self.sessions {
            let homes: Vec<CellId> = CELLS
                .into_iter()
                .filter(|cell| self.cells[cell].sessions.contains_key(session))
                .collect();
            assert_eq!(homes.len(), 1, "seed {seed}: {session} lives in {homes:?}");
            let records = &self.cells[&homes[0]].sessions[session].authority.records;
            for key in self.acked.get(&session.uuid()).into_iter().flatten() {
                assert!(
                    records.contains_key(key),
                    "seed {seed}: {session} lost acknowledged write {key}"
                );
            }
        }
    }

    /// The end state, independent of hash order: what each cell holds.
    fn digest(&self) -> String {
        let mut out = String::new();
        for (cell, state) in &self.cells {
            let mut sessions: Vec<_> = state.sessions.iter().collect();
            sessions.sort_by_key(|(id, _)| id.to_string());
            for (id, s) in sessions {
                let keys: BTreeSet<&String> = s.authority.records.keys().collect();
                out += &format!(
                    "{cell} {id} {:?} g{} f{} e{} {keys:?}\n",
                    s.placement,
                    s.generation,
                    s.authority.fence,
                    s.events.len()
                );
            }
            let mut directory: Vec<_> = state.directory.iter().collect();
            directory.sort_by_key(|(id, _)| id.to_string());
            out += &format!("{cell} directory {directory:?}\n");
            for (tenant, allowance) in &state.allowances.tenants {
                out += &format!("{cell} {tenant} {:?}\n", allowance.grants);
            }
        }
        out
    }
}

fn run(seed: u64, steps: usize) -> (String, BTreeMap<&'static str, u64>) {
    let mut world = World::new(seed);
    for _ in 0..steps {
        world.step();
    }
    world.quiesce();
    world.final_checks(seed);
    (world.digest(), world.stats)
}

#[test]
fn every_seed_keeps_one_owner_one_writer_and_every_acknowledged_write() {
    let mut totals: BTreeMap<&'static str, u64> = BTreeMap::new();
    for seed in 1..=40 {
        let (first, stats) = run(seed, 4_000);
        let (second, _) = run(seed, 4_000);
        assert_eq!(first, second, "seed {seed} does not replay");
        for (what, n) in stats {
            *totals.entry(what).or_default() += n;
        }
    }
    eprintln!("40 seeds: {totals:?}");
    // The run must have exercised what it claims to.
    for what in [
        "acked writes",
        "moves finished",
        "executor deaths",
        "driver deaths",
        "duplicated",
    ] {
        assert!(
            totals.get(what).copied().unwrap_or(0) > 20,
            "too few {what}: {totals:?}"
        );
    }
}
