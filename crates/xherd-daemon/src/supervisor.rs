//! The supervisor task owns global state: the agent registry, concurrency
//! slots, the start queue and port blocks (design §3.2). It never waits on an
//! actor: replies travel from the actor straight to the caller.

use std::collections::{BTreeSet, HashMap};

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use xherd_core::report::{Origin, Report};
use xherd_core::xap::{Refusal, XapCall, refuse};
use xherd_core::{Agent, Millis, Status};

use crate::actor::{
    self, ActionError, AgentTx, Env, Msg, ReportResult, SupEvent, TickKind, UserAction,
};

/// First port handed to agents; each live agent gets a block of 10 (`XHERD_PORT_BASE`).
pub const PORT_RANGE_START: u16 = 41000;
pub const PORT_BLOCK: u16 = 10;

pub enum SupMsg {
    Create(Box<Agent>, oneshot::Sender<Result<Agent, ActionError>>),
    Report(Report, Origin, oneshot::Sender<Option<ReportResult>>),
    /// `who` is an agent id, a name, or a unique ticket key.
    User(
        String,
        UserAction,
        oneshot::Sender<Result<Agent, ActionError>>,
    ),
    Xap(XapCall, oneshot::Sender<Result<Value, Refusal>>),
    Tick(TickKind),
    /// Live session names from the driver (R-011).
    Reconcile(Vec<String>, bool),
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
    pub async fn create(&self, a: Agent) -> Result<Agent, ActionError> {
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

    pub async fn action(&self, who: &str, a: UserAction) -> Result<Agent, ActionError> {
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
}

struct Entry {
    tx: AgentTx,
    name: String,
    ticket_key: String,
    status: Status,
    created_at: Millis,
    port_block: Option<u16>,
}

struct Supervisor {
    max_live: usize,
    agents: HashMap<String, Entry>,
    /// Sent `Start`, waiting for the status change.
    starting: BTreeSet<String>,
    env: Env,
}

/// Spawn the supervisor with existing agents (from the store).
pub fn spawn(agents: Vec<Agent>, max_live: usize, mut env: Env) -> SupervisorHandle {
    let (tx, mut rx) = mpsc::unbounded_channel::<SupMsg>();
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<SupEvent>();
    env.sup = ev_tx;
    let mut sup = Supervisor {
        max_live,
        agents: HashMap::new(),
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
    fn register(&mut self, a: Agent) {
        let entry = Entry {
            tx: actor::spawn(a.clone(), self.env.clone()),
            name: a.name.clone(),
            ticket_key: a.ticket_key.clone(),
            status: a.status,
            created_at: a.created_at,
            port_block: None,
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
            .filter(|(_, e)| e.ticket_key == who)
            .map(|(id, _)| id)
            .collect();
        match by_key.as_slice() {
            [one] => Ok(one),
            [] => Err(ActionError {
                code: "unknown_agent",
                message: format!("no agent {who:?}"),
            }),
            _ => Err(ActionError {
                code: "ambiguous_agent",
                message: format!("several agents for {who:?}; use the name"),
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
                if let Err(e) = self.env.store.put_agent(&a).await {
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
            SupMsg::Report(r, origin, reply) => match self.agents.get(&r.agent_id) {
                Some(e) => {
                    let (tx, rx) = oneshot::channel();
                    let _ = e.tx.send(Msg::Report(r, origin, Some(tx)));
                    tokio::spawn(async move {
                        let _ = reply.send(rx.await.ok());
                    });
                }
                None => {
                    tracing::warn!(agent = %r.agent_id, event = r.event.as_str(), "report for unknown agent");
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
                        let _ = self.agents[id].tx.send(Msg::User(action, reply));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            SupMsg::Xap(call, reply) => match self.agents.get(&call.agent_id) {
                Some(e) => {
                    let _ = e.tx.send(Msg::Xap(call, reply));
                }
                None => {
                    let _ = reply.send(Err(refuse("unknown_agent", "no such agent")));
                }
            },
            SupMsg::Tick(t) => {
                for e in self.agents.values() {
                    let _ = e.tx.send(Msg::Tick(t));
                }
            }
            SupMsg::Reconcile(alive, recover) => {
                for e in self.agents.values().filter(|e| e.status.is_live()) {
                    let alive = alive.contains(&e.name);
                    let _ =
                        e.tx.send(Msg::Tick(TickKind::SessionCheck { alive, recover }));
                }
            }
        }
    }

    fn on_event(&mut self, ev: SupEvent) {
        let SupEvent::StatusChanged { id, to } = ev;
        if let Some(e) = self.agents.get_mut(&id) {
            e.status = to;
            if !to.is_live() {
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
        for (_, id) in queued.into_iter().take(free) {
            let block = self.free_block();
            let e = self.agents.get_mut(&id).expect("queued agent");
            e.port_block = Some(block);
            let port_base = PORT_RANGE_START + block * PORT_BLOCK;
            let (tx, _rx) = oneshot::channel();
            let _ = e.tx.send(Msg::User(UserAction::Start { port_base }, tx));
            self.starting.insert(id);
        }
    }

    fn free_block(&self) -> u16 {
        let used: BTreeSet<u16> = self.agents.values().filter_map(|e| e.port_block).collect();
        (0..).find(|b| !used.contains(b)).expect("a free block")
    }
}
