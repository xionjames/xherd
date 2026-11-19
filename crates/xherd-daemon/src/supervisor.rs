//! The supervisor task owns global state: the agent registry, concurrency
//! slots, the start queue and port blocks (design §3.2). It never waits on an
//! actor: replies travel from the actor straight to the caller.

use std::collections::{BTreeSet, HashMap};

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use xherd_core::report::{Origin, Report};
use xherd_core::xap::{Refusal, XapCall, refuse};
use xherd_core::{Job, Millis, Status};

use crate::actor::{
    self, ActionError, AgentTx, Env, Msg, ReportResult, SupEvent, TickKind, UserAction,
};

/// First port handed to agents; each live agent gets a block of 10 (`XHERD_PORT_BASE`).
pub const PORT_RANGE_START: u16 = 41000;
pub const PORT_BLOCK: u16 = 10;
/// How many blocks fit in the range (up to port 49999).
const PORT_BLOCKS: u16 = (50000 - PORT_RANGE_START) / PORT_BLOCK;

pub enum SupMsg {
    Create(Box<Job>, oneshot::Sender<Result<Job, ActionError>>),
    Report(Report, Origin, oneshot::Sender<Option<ReportResult>>),
    /// `who` is an agent id, a name, or a unique ticket key.
    User(
        String,
        UserAction,
        oneshot::Sender<Result<Job, ActionError>>,
    ),
    Xap(XapCall, oneshot::Sender<Result<Value, Refusal>>),
    Tick(TickKind),
    /// Live session names from the driver (R-011).
    Reconcile(Vec<String>, bool),
    /// A saved agent's `maxLiveJobs` (R16); `None` once the agent is gone.
    AgentLimit(String, Option<usize>),
}

#[derive(Clone)]
pub struct SupervisorHandle {
    tx: mpsc::UnboundedSender<SupMsg>,
}

fn gone() -> ActionError {
    ActionError {
        code: "unavailable",
        message: "daemon is shutting down".into(),
    }
}

impl SupervisorHandle {
    pub async fn create(&self, a: Job) -> Result<Job, ActionError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(SupMsg::Create(Box::new(a), tx))
            .map_err(|_| gone())?;
        rx.await.map_err(|_| gone())?
    }

    /// `None` when the agent is unknown.
    pub async fn report(&self, r: Report, origin: Origin) -> Option<ReportResult> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(SupMsg::Report(r, origin, tx)).ok()?;
        rx.await.ok().flatten()
    }

    pub async fn action(&self, who: &str, a: UserAction) -> Result<Job, ActionError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(SupMsg::User(who.to_string(), a, tx))
            .map_err(|_| gone())?;
        rx.await.map_err(|_| gone())?
    }

    pub async fn xap(&self, call: XapCall) -> Result<Value, Refusal> {
        let gone = || refuse("unavailable", "daemon is shutting down");
        let (tx, rx) = oneshot::channel();
        self.tx.send(SupMsg::Xap(call, tx)).map_err(|_| gone())?;
        rx.await.map_err(|_| gone())?
    }

    pub fn tick(&self, t: TickKind) {
        let _ = self.tx.send(SupMsg::Tick(t));
    }

    pub fn reconcile(&self, alive: Vec<String>, recover: bool) {
        let _ = self.tx.send(SupMsg::Reconcile(alive, recover));
    }

    /// Tell the supervisor an agent's live-job limit (R16).
    pub fn agent_limit(&self, id: &str, limit: Option<usize>) {
        let _ = self.tx.send(SupMsg::AgentLimit(id.to_string(), limit));
    }
}

struct Entry {
    tx: AgentTx,
    /// The saved agent the job came from (`Job.agent.id`).
    agent: Option<String>,
    name: String,
    ticket_key: Option<String>,
    status: Status,
    created_at: Millis,
    /// Index of the job's port block: base = `PORT_RANGE_START + block * PORT_BLOCK`.
    port_block: Option<u16>,
    /// An action of the job runs: the port block is kept (R12a).
    actions_running: bool,
}

struct Supervisor {
    max_live: usize,
    agents: HashMap<String, Entry>,
    /// `maxLiveJobs` of each saved agent, by id (R16).
    agent_limits: HashMap<String, usize>,
    /// Sent `Start`, waiting for the status change.
    starting: BTreeSet<String>,
    env: Env,
}

/// Spawn the supervisor with existing jobs and the saved agents' limits.
pub fn spawn(
    agents: Vec<Job>,
    agent_limits: HashMap<String, usize>,
    max_live: usize,
    mut env: Env,
) -> SupervisorHandle {
    let (tx, mut rx) = mpsc::unbounded_channel::<SupMsg>();
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<SupEvent>();
    env.sup = ev_tx;
    let mut sup = Supervisor {
        max_live,
        agents: HashMap::new(),
        agent_limits,
        starting: BTreeSet::new(),
        env,
    };
    for a in agents {
        sup.register(a);
    }
    tokio::spawn(async move {
        sup.fill_slots();
        loop {
            tokio::select! {
                biased;
                Some(ev) = ev_rx.recv() => sup.on_event(ev),
                msg = rx.recv() => match msg {
                    Some(m) => sup.on_msg(m).await,
                    None => break,
                },
            }
        }
    });
    SupervisorHandle { tx }
}

impl Supervisor {
    fn register(&mut self, a: Job) {
        let entry = Entry {
            tx: actor::spawn(a.clone(), self.env.clone()),
            agent: a.agent().map(|g| g.id.clone()),
            name: a.name.clone(),
            ticket_key: a.ticket_key().map(str::to_string),
            status: a.status,
            created_at: a.created_at,
            // A block survives a restart through `Job.port_base` (R12a).
            port_block: a.port_base.and_then(block_of),
            actions_running: xherd_core::action::running(&a),
        };
        self.agents.insert(a.id, entry);
    }

    fn resolve(&self, who: &str) -> Result<&str, ActionError> {
        if let Some((id, _)) = self.agents.get_key_value(who) {
            return Ok(id);
        }
        if let Some((id, _)) = self.agents.iter().find(|(_, e)| e.name == who) {
            return Ok(id);
        }
        let by_key: Vec<&String> = self
            .agents
            .iter()
            .filter(|(_, e)| e.ticket_key.as_deref() == Some(who))
            .map(|(id, _)| id)
            .collect();
        match by_key.as_slice() {
            [one] => Ok(one),
            [] => Err(ActionError {
                code: "unknown_job",
                message: format!("no job {who:?}"),
            }),
            _ => Err(ActionError {
                code: "ambiguous_job",
                message: format!("several jobs for {who:?}; use the name"),
            }),
        }
    }

    async fn on_msg(&mut self, m: SupMsg) {
        match m {
            SupMsg::Create(a, reply) => {
                if self.agents.values().any(|e| e.name == a.name) {
                    let _ = reply.send(Err(ActionError {
                        code: "name_taken",
                        message: format!("agent {} exists", a.name),
                    }));
                    return;
                }
                if let Err(e) = self.env.store.put_job(&a).await {
                    let _ = reply.send(Err(ActionError {
                        code: "store",
                        message: e.to_string(),
                    }));
                    return;
                }
                self.register((*a).clone());
                let _ = reply.send(Ok(*a));
                self.fill_slots();
            }
            SupMsg::Report(r, origin, reply) => match self.agents.get(&r.job_id) {
                Some(e) => {
                    let (tx, rx) = oneshot::channel();
                    let _ = e.tx.send(Msg::Report(r, origin, Some(tx)));
                    tokio::spawn(async move {
                        let _ = reply.send(rx.await.ok());
                    });
                }
                None => {
                    tracing::warn!(agent = %r.job_id, event = r.event.as_str(), "report for unknown agent");
                    let _ = reply.send(None);
                }
            },
            SupMsg::User(who, action, reply) => {
                if matches!(action, UserAction::Start { .. }) {
                    let _ = reply.send(Err(ActionError {
                        code: "not_allowed",
                        message: "agents start when a slot is free".into(),
                    }));
                    return;
                }
                match self.resolve(&who) {
                    Ok(id) => {
                        let id = id.to_string();
                        self.send_user(&id, action, reply);
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            SupMsg::Xap(call, reply) => match self.agents.get(&call.job_id) {
                Some(e) => {
                    let _ = e.tx.send(Msg::Xap(call, reply));
                }
                None => {
                    let _ = reply.send(Err(refuse("unknown_job", "no such job")));
                }
            },
            SupMsg::Tick(t) => {
                for e in self.agents.values() {
                    let _ = e.tx.send(Msg::Tick(t));
                }
            }
            SupMsg::Reconcile(alive, recover) => {
                // A job that is not live but has a running action is checked
                // too: the actor adopts or loses its action sessions (R12).
                for e in self
                    .agents
                    .values()
                    .filter(|e| e.status.is_live() || e.actions_running)
                {
                    let alive = alive.contains(&e.name);
                    let _ =
                        e.tx.send(Msg::Tick(TickKind::SessionCheck { alive, recover }));
                }
                if recover {
                    // A job's own session is its name: those are not action
                    // sessions, whatever they look like (`a-act-b`).
                    let names: BTreeSet<&str> =
                        self.agents.values().map(|e| e.name.as_str()).collect();
                    let others: std::sync::Arc<Vec<String>> = std::sync::Arc::new(
                        alive
                            .iter()
                            .filter(|s| !names.contains(s.as_str()))
                            .cloned()
                            .collect(),
                    );
                    for e in self.agents.values() {
                        let _ = e.tx.send(Msg::Sessions(others.clone()));
                    }
                }
            }
            SupMsg::AgentLimit(id, limit) => {
                match limit {
                    Some(n) => self.agent_limits.insert(id, n),
                    None => self.agent_limits.remove(&id),
                };
                self.fill_slots();
            }
        }
    }

    fn on_event(&mut self, ev: SupEvent) {
        let (id, to) = match ev {
            SupEvent::StatusChanged { id, to } => (id, to),
            SupEvent::ActionsRunning { id, running } => {
                if let Some(e) = self.agents.get_mut(&id) {
                    e.actions_running = running;
                    if !running && !e.status.is_live() {
                        e.port_block = None;
                    }
                }
                return;
            }
            SupEvent::AgentChanged { id, agent } => {
                if let Some(e) = self.agents.get_mut(&id) {
                    e.agent = agent;
                }
                return;
            }
        };
        if let Some(e) = self.agents.get_mut(&id) {
            e.status = to;
            if !to.is_live() && !e.actions_running {
                e.port_block = None;
            }
        }
        if to != Status::Queued {
            self.starting.remove(&id);
        }
        self.fill_slots();
    }

    /// Start queued agents, oldest first, while live agents < cap (R-006).
    fn fill_slots(&mut self) {
        let live =
            self.agents.values().filter(|e| e.status.is_live()).count() + self.starting.len();
        let free = self.max_live.saturating_sub(live);
        if free == 0 {
            return;
        }
        let mut queued: Vec<(Millis, String)> = self
            .agents
            .iter()
            .filter(|(id, e)| e.status == Status::Queued && !self.starting.contains(*id))
            .map(|(id, e)| (e.created_at, id.clone()))
            .collect();
        queued.sort();
        // Jobs of an agent that is at its `maxLiveJobs` wait; they do not hold
        // a slot back from other jobs (R16).
        let mut per_agent: HashMap<String, usize> = HashMap::new();
        for (id, e) in &self.agents {
            if let Some(a) = &e.agent
                && (e.status.is_live() || self.starting.contains(id))
            {
                *per_agent.entry(a.clone()).or_default() += 1;
            }
        }
        let mut chosen = Vec::new();
        for (_, id) in queued {
            if chosen.len() == free {
                break;
            }
            if let Some(a) = &self.agents[&id].agent
                && let Some(&limit) = self.agent_limits.get(a)
            {
                let n = per_agent.entry(a.clone()).or_default();
                if *n >= limit {
                    continue;
                }
                *n += 1;
            }
            chosen.push(id);
        }
        for id in chosen {
            // No free block: the job stays queued until one is released.
            let Some(block) = self.free_block() else {
                tracing::warn!(agent = %id, "no_ports: every port block is in use");
                break;
            };
            let e = self.agents.get_mut(&id).expect("queued agent");
            e.port_block = Some(block);
            let port_base = PORT_RANGE_START + block * PORT_BLOCK;
            let (tx, _rx) = oneshot::channel();
            let _ = e.tx.send(Msg::User(UserAction::Start { port_base }, tx));
            self.starting.insert(id);
        }
    }

    /// Hand a user action to the job's actor. A Run gets the job's port block
    /// (R12a): the one it holds, else a free one reserved now, refused
    /// `no_ports` when none is free. A reserved block the actor does not use
    /// (the Run was refused) is released again.
    fn send_user(
        &mut self,
        id: &str,
        mut action: UserAction,
        reply: oneshot::Sender<Result<Job, ActionError>>,
    ) {
        let mut reserved = false;
        if let UserAction::ActionRun { port_base, .. } = &mut action {
            let e = &self.agents[id];
            let block = match e.port_block {
                Some(b) => Some(b),
                None => {
                    reserved = true;
                    self.free_block()
                }
            };
            let Some(block) = block else {
                let _ = reply.send(Err(ActionError {
                    code: "no_ports",
                    message: "every port block is in use".into(),
                }));
                return;
            };
            *port_base = Some(PORT_RANGE_START + block * PORT_BLOCK);
            if reserved {
                self.agents.get_mut(id).expect("agent").port_block = Some(block);
            }
        }
        let e = &self.agents[id];
        if !reserved {
            let _ = e.tx.send(Msg::User(action, reply));
            return;
        }
        let (tx, rx) = oneshot::channel();
        let _ = e.tx.send(Msg::User(action, tx));
        let (sup, id) = (self.env.sup.clone(), id.to_string());
        tokio::spawn(async move {
            let res = rx.await.unwrap_or_else(|_| {
                Err(ActionError {
                    code: "internal",
                    message: "the job is gone".into(),
                })
            });
            if res.is_err() {
                let _ = sup.send(SupEvent::ActionsRunning { id, running: false });
            }
            let _ = reply.send(res);
        });
    }

    /// The lowest unused block, `None` when the range is exhausted.
    fn free_block(&self) -> Option<u16> {
        first_free(self.agents.values().filter_map(|e| e.port_block))
    }
}

/// The block index of a job's `port_base`; `None` outside the range.
fn block_of(port_base: u16) -> Option<u16> {
    let block = port_base.checked_sub(PORT_RANGE_START)? / PORT_BLOCK;
    (block < PORT_BLOCKS).then_some(block)
}

fn first_free(used: impl Iterator<Item = u16>) -> Option<u16> {
    let used: BTreeSet<u16> = used.collect();
    (0..PORT_BLOCKS).find(|b| !used.contains(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_comes_back_from_the_job_port_base() {
        assert_eq!(block_of(PORT_RANGE_START), Some(0));
        assert_eq!(block_of(PORT_RANGE_START + 3 * PORT_BLOCK), Some(3));
        assert_eq!(block_of(PORT_RANGE_START - 1), None);
        assert_eq!(block_of(60000), None);
    }

    #[test]
    fn the_lowest_free_block_is_taken_and_none_is_left_when_all_are_used() {
        assert_eq!(first_free([0, 1, 3].into_iter()), Some(2));
        assert_eq!(first_free(std::iter::empty()), Some(0));
        assert_eq!(first_free(0..PORT_BLOCKS), None);
    }
}
