//! The chat view's events and the mapper from a runtime transcript to them
//! (#202, R2, R3). Pure: no IO and no clock; the daemon's actor reads the
//! transcript, feeds lines to [`Mapper::line`], stamps `seq`/`at`/`end` and logs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::job::Millis;

/// Longest `Tool.summary`, in characters.
const SUMMARY_MAX: usize = 120;
/// Longest raw line kept in an `Unknown` body.
const UNKNOWN_MAX: usize = 4096;
/// Longest tool result kept, in bytes.
const RESULT_MAX: usize = 4096;
/// The tool whose use is a question for the person.
pub const QUESTION_TOOL: &str = "AskUserQuestion";

/// One entry of a job's chat log. `end` is the transcript offset just after
/// the line the event came from (used to resume after a restart).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEvent {
    pub seq: u64,
    pub at: Millis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u64>,
    #[serde(flatten)]
    pub body: ChatBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ChatBody {
    User {
        text: String,
        source: Source,
    },
    Assistant {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        summary: String,
        status: ToolStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    Plan {
        items: Vec<PlanItem>,
    },
    /// Live only: never logged.
    Status {
        state: LiveState,
    },
    Notice {
        notice: NoticeKind,
        text: String,
    },
    Permission {
        id: String,
        tool: String,
        summary: String,
        options: Vec<PermOption>,
        state: Resolution,
    },
    Question {
        id: String,
        questions: Vec<Question>,
        answerable: bool,
        state: Resolution,
    },
    /// A line the mapper did not understand (raw cut to 4 KiB). Logged, never
    /// sent to clients.
    Unknown {
        raw: String,
    },
}

/// Who wrote a user message: the person, or a command xherd typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    User,
    Xherd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolStatus {
    Running,
    Ok,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LiveState {
    Working,
    Idle,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoticeKind {
    NewSession,
    Resume,
    Ask,
    Compaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    pub text: String,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermOption {
    Allow,
    AllowAlways,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnsweredFrom {
    Chat,
    Inbox,
    Terminal,
}

/// How a permission or question card ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Resolution {
    Waiting,
    Answered {
        answer: String,
        from: AnsweredFrom,
    },
    /// Answered in the runtime's own prompt, or its result arrived with no
    /// xherd answer.
    InTerminal,
    TimedOut,
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub question: String,
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub multi_select: bool,
    pub options: Vec<QuestionOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

/// What a later `tool_result` needs to re-emit a tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolMeta {
    pub name: String,
    pub summary: String,
    pub path: Option<String>,
}

/// Turns transcript lines into chat bodies. It remembers the tool and
/// question ids that are in the log, so a later `tool_result` can update
/// them (and is dropped when its id is not known).
#[derive(Debug, Clone, Default)]
pub struct Mapper {
    /// Prefix of the commands xherd types (`/xherd:`), to tell them from the
    /// person's own messages.
    pub prefix: Option<String>,
    pub tools: BTreeMap<String, ToolMeta>,
    pub questions: BTreeMap<String, Vec<Question>>,
}

impl Mapper {
    pub fn new(prefix: Option<String>) -> Mapper {
        Mapper {
            prefix,
            ..Mapper::default()
        }
    }

    /// The bodies one `claude-jsonl` transcript line gives (none for a line
    /// that is not chat).
    pub fn line(&mut self, line: &str) -> Vec<ChatBody> {
        claude::map(line, self)
    }

    /// Drops ids whose events were folded out of the log.
    pub fn forget<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) {
        for id in ids {
            self.tools.remove(id);
            self.questions.remove(id);
        }
    }
}

/// One line for a tool call: `Edit src/a.rs`, `Bash: cargo test`, `Grep "pat"`,
/// else the tool name. At most 120 characters.
pub fn summary(tool: &str, input: &Value) -> String {
    let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or("").trim();
    let line = match tool {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "Read"
            if !text("file_path").is_empty() =>
        {
            format!("{tool} {}", text("file_path"))
        }
        "Bash" if !text("command").is_empty() => {
            format!("Bash: {}", text("command").lines().next().unwrap_or(""))
        }
        "Grep" if !text("pattern").is_empty() => format!("Grep \"{}\"", text("pattern")),
        _ => tool.to_string(),
    };
    cut(&line.replace('\n', " "), SUMMARY_MAX)
}

/// The first `max` bytes of `s`, cut at a character boundary.
fn head(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// `s` cut to `max` characters, ending in `…` when it was longer.
fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The `claude-jsonl` format: Claude Code's session transcript.
mod claude {
    use super::*;

    pub(super) fn map(line: &str, m: &mut Mapper) -> Vec<ChatBody> {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return vec![unknown(line)];
        };
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        if flag("isSidechain") || flag("isMeta") || flag("isCompactSummary") {
            return Vec::new();
        }
        if kind == "system" {
            return match v.get("subtype").and_then(Value::as_str) {
                Some("compact_boundary") => vec![ChatBody::Notice {
                    notice: NoticeKind::Compaction,
                    text: "Conversation compacted".to_string(),
                }],
                _ => Vec::new(),
            };
        }
        if !matches!(kind, "user" | "assistant") {
            return Vec::new();
        }
        let Some(content) = v.pointer("/message/content") else {
            return vec![unknown(line)];
        };
        let blocks: Vec<Value> = match content {
            Value::String(s) => vec![serde_json::json!({"type": "text", "text": s})],
            Value::Array(a) => a.clone(),
            _ => return vec![unknown(line)],
        };
        let mut out = Vec::new();
        let mut recognised = false;
        let mut user_text = Vec::new();
        for b in &blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("text") => {
                    recognised = true;
                    let text = b.get("text").and_then(Value::as_str).unwrap_or("");
                    if kind == "assistant" {
                        if !text.trim().is_empty() {
                            out.push(ChatBody::Assistant {
                                text: text.to_string(),
                            });
                        }
                    } else {
                        user_text.push(text);
                    }
                }
                Some("tool_use") => {
                    if let Some(body) = tool_use(b, m) {
                        recognised = true;
                        out.push(body);
                    }
                }
                Some("tool_result") => {
                    recognised = true;
                    out.extend(tool_result(b, m));
                }
                Some("thinking" | "redacted_thinking" | "image") => recognised = true,
                _ => {}
            }
        }
        let joined = user_text.join("\n");
        if !joined.trim().is_empty() {
            out.insert(
                0,
                ChatBody::User {
                    source: source_of(&joined, m.prefix.as_deref()),
                    text: joined,
                },
            );
        }
        if !recognised {
            return vec![unknown(line)];
        }
        out
    }

    /// Claude Code records a slash command as
    /// `<command-message>xherd:design</command-message>\n<command-name>/xherd:design</command-name>…`.
    fn source_of(text: &str, prefix: Option<&str>) -> Source {
        let Some(prefix) = prefix.filter(|p| !p.is_empty()) else {
            return Source::User;
        };
        let mut t = text.trim_start();
        if let Some(rest) = t.strip_prefix("<command-message>")
            && let Some((_, after)) = rest.split_once("</command-message>")
        {
            t = after.trim_start();
        }
        let named = t
            .strip_prefix("<command-name>")
            .is_some_and(|r| r.starts_with(prefix));
        if named || t.starts_with(prefix) {
            Source::Xherd
        } else {
            Source::User
        }
    }

    fn unknown(line: &str) -> ChatBody {
        ChatBody::Unknown {
            raw: head(line, UNKNOWN_MAX).to_string(),
        }
    }

    /// A `tool_use` block: a tool call, the plan, or a question.
    fn tool_use(b: &Value, m: &mut Mapper) -> Option<ChatBody> {
        let id = b.get("id").and_then(Value::as_str)?.to_string();
        let name = b.get("name").and_then(Value::as_str)?.to_string();
        let input = b.get("input").unwrap_or(&Value::Null);
        match name.as_str() {
            "TodoWrite" => Some(ChatBody::Plan {
                items: input
                    .get("todos")
                    .and_then(Value::as_array)
                    .map(|todos| todos.iter().map(plan_item).collect())
                    .unwrap_or_default(),
            }),
            QUESTION_TOOL => {
                let questions = questions_of(input);
                m.questions.insert(id.clone(), questions.clone());
                Some(ChatBody::Question {
                    id,
                    answerable: answerable(&questions),
                    questions,
                    state: Resolution::Waiting,
                })
            }
            _ => {
                let path = matches!(
                    name.as_str(),
                    "Edit" | "Write" | "MultiEdit" | "NotebookEdit"
                )
                .then(|| input.get("file_path").and_then(Value::as_str))
                .flatten()
                .map(str::to_string);
                let meta = ToolMeta {
                    summary: summary(&name, input),
                    name,
                    path,
                };
                m.tools.insert(id.clone(), meta.clone());
                Some(ChatBody::Tool {
                    id,
                    name: meta.name,
                    summary: meta.summary,
                    status: ToolStatus::Running,
                    result: None,
                    path: meta.path,
                })
            }
        }
    }

    /// A `tool_result` block: the finished tool, or the question's end. Nothing
    /// when the id is not in the log.
    fn tool_result(b: &Value, m: &Mapper) -> Option<ChatBody> {
        let id = b.get("tool_use_id").and_then(Value::as_str)?.to_string();
        if let Some(questions) = m.questions.get(&id) {
            return Some(ChatBody::Question {
                id,
                answerable: answerable(questions),
                questions: questions.clone(),
                state: Resolution::InTerminal,
            });
        }
        let meta = m.tools.get(&id)?;
        let failed = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
        let text = match b.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter_map(|x| x.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        Some(ChatBody::Tool {
            id,
            name: meta.name.clone(),
            summary: meta.summary.clone(),
            status: if failed {
                ToolStatus::Error
            } else {
                ToolStatus::Ok
            },
            result: (!text.is_empty()).then(|| head(&text, RESULT_MAX).to_string()),
            path: meta.path.clone(),
        })
    }

    fn plan_item(todo: &Value) -> PlanItem {
        let text = |k: &str| todo.get(k).and_then(Value::as_str).unwrap_or("");
        PlanItem {
            text: text("content").to_string(),
            status: match text("status") {
                "in_progress" => PlanStatus::InProgress,
                "completed" => PlanStatus::Completed,
                _ => PlanStatus::Pending,
            },
        }
    }

    fn questions_of(input: &Value) -> Vec<Question> {
        let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        input
            .get("questions")
            .and_then(Value::as_array)
            .map(|qs| {
                qs.iter()
                    .map(|q| Question {
                        question: text(q, "question"),
                        header: text(q, "header"),
                        multi_select: q
                            .get("multiSelect")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        options: q
                            .get("options")
                            .and_then(Value::as_array)
                            .map(|os| {
                                os.iter()
                                    .map(|o| QuestionOption {
                                        label: text(o, "label"),
                                        description: text(o, "description"),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Only one single-choice question can be answered with a key press.
    fn answerable(questions: &[Question]) -> bool {
        matches!(questions, [q] if !q.multi_select)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FIXTURE: &str = include_str!("../tests/fixtures/claude-transcript.jsonl");

    fn run(prefix: Option<&str>, text: &str) -> Vec<ChatBody> {
        let mut m = Mapper::new(prefix.map(str::to_string));
        text.lines().flat_map(|l| m.line(l)).collect()
    }

    #[test]
    fn the_fixture_gives_messages_and_skips_the_rest() {
        let bodies = run(Some("/xherd:"), FIXTURE);
        assert_eq!(
            bodies,
            vec![
                ChatBody::User {
                    text: "<command-message>xherd:commit</command-message>\n<command-name>/xherd:commit</command-name>\n<command-args>C05</command-args>".into(),
                    source: Source::Xherd,
                },
                ChatBody::Assistant {
                    text: "Starting T06: I'm reading the design.".into()
                },
                ChatBody::User {
                    text: "Please also check the docs".into(),
                    source: Source::User,
                },
                ChatBody::Assistant {
                    text: "Done.".into()
                },
                tool("toolu_1", "Bash", "Bash: cargo test", ToolStatus::Running, None, None),
                tool(
                    "toolu_1",
                    "Bash",
                    "Bash: cargo test",
                    ToolStatus::Ok,
                    Some("test result: ok. 3 passed"),
                    None
                ),
                tool("toolu_2", "Bash", "Bash: cargo clippy", ToolStatus::Running, None, None),
                tool(
                    "toolu_2",
                    "Bash",
                    "Bash: cargo clippy",
                    ToolStatus::Error,
                    Some("error: boom"),
                    None
                ),
                tool("toolu_3", "Edit", "Edit src/a.rs", ToolStatus::Running, None, Some("src/a.rs")),
                tool(
                    "toolu_3",
                    "Edit",
                    "Edit src/a.rs",
                    ToolStatus::Ok,
                    Some("The file src/a.rs has been updated."),
                    Some("src/a.rs")
                ),
                ChatBody::Plan {
                    items: vec![
                        PlanItem {
                            text: "Write the mapper".into(),
                            status: PlanStatus::InProgress
                        },
                        PlanItem {
                            text: "Test it".into(),
                            status: PlanStatus::Pending
                        },
                        PlanItem {
                            text: "Read the design".into(),
                            status: PlanStatus::Completed
                        },
                    ]
                },
                ChatBody::Question {
                    id: "toolu_5".into(),
                    questions: vec![store_question()],
                    answerable: true,
                    state: Resolution::Waiting,
                },
                ChatBody::Question {
                    id: "toolu_5".into(),
                    questions: vec![store_question()],
                    answerable: true,
                    state: Resolution::InTerminal,
                },
                ChatBody::Notice {
                    notice: NoticeKind::Compaction,
                    text: "Conversation compacted".into()
                },
            ]
        );
    }

    fn tool(
        id: &str,
        name: &str,
        summary: &str,
        status: ToolStatus,
        result: Option<&str>,
        path: Option<&str>,
    ) -> ChatBody {
        ChatBody::Tool {
            id: id.into(),
            name: name.into(),
            summary: summary.into(),
            status,
            result: result.map(str::to_string),
            path: path.map(str::to_string),
        }
    }

    fn store_question() -> Question {
        Question {
            question: "Which store should the cache use?".into(),
            header: "Store".into(),
            multi_select: false,
            options: vec![
                QuestionOption {
                    label: "SQLite".into(),
                    description: "One file, already a dependency".into(),
                },
                QuestionOption {
                    label: "Memory".into(),
                    description: "Lost at restart".into(),
                },
            ],
        }
    }

    #[test]
    fn results_of_unknown_tools_are_dropped_and_forgotten_ids_too() {
        let call = json!({"type":"assistant","message":{"content":[
            {"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"x"}}]}})
        .to_string();
        let result = |text: String| {
            json!({"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":"t1","content":text}]}})
            .to_string()
        };
        let mut m = Mapper::default();
        assert_eq!(m.line(&call).len(), 1);
        let ChatBody::Tool {
            result: Some(r), ..
        } = &m.line(&result("é".repeat(RESULT_MAX)))[0]
        else {
            panic!("tool expected");
        };
        assert!(r.len() <= RESULT_MAX);
        m.forget(["t1"]);
        assert_eq!(m.line(&result("late".into())), vec![]);
    }

    #[test]
    fn only_one_single_choice_question_is_answerable() {
        let ask = |qs: Value| {
            json!({"type":"assistant","message":{"content":[
                {"type":"tool_use","id":"q","name":QUESTION_TOOL,"input":{"questions":qs}}]}})
            .to_string()
        };
        let one = json!({"question":"a?","options":[{"label":"x"}]});
        let multi = json!({"question":"b?","multiSelect":true,"options":[{"label":"x"}]});
        let answerable = |qs: Value| {
            let mut m = Mapper::default();
            let ChatBody::Question { answerable, .. } = m.line(&ask(qs)).remove(0) else {
                panic!("question expected");
            };
            answerable
        };
        assert!(answerable(json!([one.clone()])));
        assert!(!answerable(json!([multi])));
        assert!(!answerable(json!([one.clone(), one])));
    }

    #[test]
    fn a_tool_use_without_an_id_is_unknown() {
        let mut m = Mapper::default();
        let line =
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}})
                .to_string();
        assert!(matches!(m.line(&line)[..], [ChatBody::Unknown { .. }]));
    }

    #[test]
    fn a_message_that_is_not_a_command_is_the_users() {
        let line =
            json!({"type":"user","message":{"role":"user","content":"/commit now"}}).to_string();
        let mut m = Mapper::new(Some("/xherd:".into()));
        assert!(matches!(
            m.line(&line)[0],
            ChatBody::User {
                source: Source::User,
                ..
            }
        ));
        let mut none = Mapper::default();
        let cmd =
            json!({"type":"user","message":{"role":"user","content":"/xherd:next"}}).to_string();
        assert!(matches!(
            none.line(&cmd)[0],
            ChatBody::User {
                source: Source::User,
                ..
            }
        ));
        assert!(matches!(
            m.line(&cmd)[0],
            ChatBody::User {
                source: Source::Xherd,
                ..
            }
        ));
    }

    #[test]
    fn sidechain_meta_and_other_types_are_skipped() {
        let mut m = Mapper::default();
        for line in [
            json!({"type":"file-history-snapshot","snapshot":{}}),
            json!({"type":"summary","summary":"x"}),
            json!({"type":"user","isMeta":true,"message":{"content":"hidden"}}),
            json!({"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"sub"}]}}),
            json!({"type":"assistant","message":{"content":[{"type":"thinking","thinking":""}]}}),
            json!({"type":"system","subtype":"turn_duration"}),
        ] {
            assert_eq!(m.line(&line.to_string()), vec![], "{line}");
        }
    }

    #[test]
    fn what_cannot_be_read_is_unknown_and_never_panics() {
        let mut m = Mapper::default();
        assert!(matches!(m.line("not json {"), v if matches!(v[..], [ChatBody::Unknown { .. }])));
        let no_content = json!({"type":"user","message":{"role":"user"}}).to_string();
        assert!(matches!(
            m.line(&no_content)[..],
            [ChatBody::Unknown { .. }]
        ));
        let odd =
            json!({"type":"assistant","message":{"content":[{"type":"hologram"}]}}).to_string();
        assert!(matches!(m.line(&odd)[..], [ChatBody::Unknown { .. }]));
        let long = format!("é{}", "é".repeat(UNKNOWN_MAX));
        let ChatBody::Unknown { raw } = &m.line(&long)[0] else {
            panic!("unknown expected");
        };
        assert!(raw.len() <= UNKNOWN_MAX);
    }

    #[test]
    fn summaries_are_one_short_line() {
        assert_eq!(
            summary("Edit", &json!({"file_path":"src/a.rs"})),
            "Edit src/a.rs"
        );
        assert_eq!(summary("Read", &json!({"file_path":"x"})), "Read x");
        assert_eq!(
            summary("Bash", &json!({"command":"cargo test\nsecond line"})),
            "Bash: cargo test"
        );
        assert_eq!(summary("Grep", &json!({"pattern":"pat"})), "Grep \"pat\"");
        assert_eq!(summary("WebSearch", &json!({"query":"q"})), "WebSearch");
        assert_eq!(summary("Edit", &json!({})), "Edit");
        let long = summary("Bash", &json!({"command":"x".repeat(500)}));
        assert_eq!(long.chars().count(), SUMMARY_MAX);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn events_serialize_flat_and_camel_case() {
        let e = ChatEvent {
            seq: 1,
            at: 5,
            session: None,
            end: Some(9),
            body: ChatBody::Permission {
                id: "p1".into(),
                tool: "Bash".into(),
                summary: "Bash: ls".into(),
                options: vec![PermOption::Allow, PermOption::AllowAlways],
                state: Resolution::Answered {
                    answer: "allow".into(),
                    from: AnsweredFrom::Chat,
                },
            },
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(
            v,
            json!({"seq":1,"at":5,"end":9,"kind":"permission","id":"p1","tool":"Bash",
                "summary":"Bash: ls","options":["allow","allow-always"],
                "state":{"state":"answered","answer":"allow","from":"chat"}})
        );
        assert_eq!(serde_json::from_value::<ChatEvent>(v).unwrap(), e);
    }
}
