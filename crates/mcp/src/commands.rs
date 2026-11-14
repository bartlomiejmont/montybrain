//! The `command` and `connections` tools, and the `waits_for` / `related`
//! fields: rendering and reference resolution on top of
//! `montybrain_core::command` and `montybrain_core::refs`.

use montybrain_core::command::{self, Context, Preview, Refusal, State, resolve};
use montybrain_core::refs::{
    self, RefTarget, build_graph, connection_line, neighbours, wait_chain,
};
use montybrain_core::{Snapshot, TaskLocation, TaskStatus, Vault};

use crate::render;

/// Every error of `command` starts with this.
pub const NOTHING: &str = "Nothing was changed:";

fn fence(lines: &[String]) -> String {
    format!("```markdown\n{}\n```", lines.join("\n"))
}

/// The plan as Markdown: what would be written, where, and the exact lines.
pub fn render_preview(p: &Preview, root: &std::path::Path) -> String {
    let mut out = String::new();
    out.push_str(&format!("Preview (nothing written yet): {}\n", p.reading));
    if let Some(plan) = &p.plan {
        for (i, s) in plan.steps.iter().enumerate() {
            out.push_str(&format!(
                "\n{}. {}\nFile: {}\n",
                i + 1,
                s.summary,
                render::abs_path(root, std::path::Path::new(&s.file))
            ));
            if !s.before.is_empty() {
                out.push_str("Now:\n");
                out.push_str(&fence(&s.before));
                out.push_str("\nAfter:\n");
            }
            out.push_str(&fence(&s.lines));
            out.push('\n');
        }
    }
    for w in &p.warnings {
        out.push_str(&format!("\nWarning: {w}"));
    }
    if !p.similar.is_empty() {
        out.push_str("\nSimilar open tasks (check for a duplicate):");
        for s in &p.similar {
            out.push_str(&format!("\n- ^{} {} ({})", s.id, s.title, s.place.label));
        }
    }
    match &p.plan {
        Some(_) if p.typo => out.push_str(
            "\n\nThis plan rests on a typo correction, so `apply` will refuse it: send the text again \
with the exact slug.",
        ),
        Some(plan) => out.push_str(&format!(
            "\n\nTo write exactly this plan, call `command` again with the same `text`, `apply: true` \
and the `plan` token from the structured result (field `plan`; here: plan: \"{}\"). If the vault changes \
in between, nothing is written and you get the new preview.",
            command::plan_token(plan)
        )),
        None => {}
    }
    out
}

/// A refusal as one error starting with [`NOTHING`].
pub fn refusal_text(r: &Refusal) -> String {
    let choices = |p: &Preview| -> String {
        if p.choices.is_empty() {
            return String::new();
        }
        let list: Vec<String> = p
            .choices
            .iter()
            .map(|c| format!("{} ({})", c.choice.insert, c.choice.label))
            .collect();
        format!(" Possible: {}.", list.join(", "))
    };
    match r {
        Refusal::Invalid { preview } if preview.state == State::Agent => format!(
            "{NOTHING} requests starting with `?` are for the agent in the MontyBrain app; this MCP \
server never calls a model. Use the commands or the structured tools instead."
        ),
        Refusal::Invalid { preview } if preview.state == State::Empty => {
            format!("{NOTHING} `text` is empty.")
        }
        Refusal::Invalid { preview } => format!(
            "{NOTHING} {}{}",
            if preview.problems.is_empty() {
                "there is nothing to write.".to_string()
            } else {
                preview.problems.join(" ")
            },
            choices(preview)
        ),
        Refusal::Typo { preview } => format!(
            "{NOTHING} {} Use the exact slug.{}",
            preview.warnings.join(" "),
            choices(preview)
        ),
        Refusal::Stale { .. } => format!("{NOTHING} the vault changed meanwhile; preview again."),
        Refusal::Failed { message } => format!("{NOTHING} {message}"),
    }
}

/// `command`: preview (default) or write the previewed plan.
///
/// Never writes anything for a preview (no `normalize` either). `apply: true`
/// needs the `plan` token of a preview of the same text, and writes only when
/// the plan rebuilt from the vault now has that token; a first call with
/// `apply: true` and no token is refused and returns the preview.
pub fn command(
    vault: &Vault,
    text: &str,
    apply: bool,
    token: Option<&str>,
    read_only: bool,
) -> Reply {
    let snap = match vault.load() {
        Ok(s) => s,
        Err(e) => return Reply::err(format!("{NOTHING} {e}")),
    };
    let ctx = Context {
        vault: Some(vault),
        ..Context::new(vault.now().date())
    };
    let p = command::preview(&snap, text, &ctx);
    let Some(plan) = p.plan.clone() else {
        return Reply::err(refusal_text(&Refusal::Invalid {
            preview: Box::new(p),
        }));
    };
    let structured = |p: &command::Preview, plan: &command::Plan| {
        serde_json::json!({
            "plan": if p.typo { serde_json::Value::Null } else { command::plan_token(plan).into() },
            "state": p.state,
            "typo": p.typo,
            "written": false,
        })
    };
    if !apply {
        return Reply {
            text: render_preview(&p, vault.root()),
            structured: Some(structured(&p, &plan)),
            is_error: false,
        };
    }
    if read_only {
        return Reply::err(format!(
            "{NOTHING} read-only run: `command` with `apply: true` is disabled because this session \
was started with {}=1. Only the preview works.",
            crate::server::ENV_READONLY
        ));
    }
    if p.typo {
        return Reply::err(refusal_text(&Refusal::Typo {
            preview: Box::new(p),
        }));
    }
    let now = command::plan_token(&plan);
    match token.map(str::trim).filter(|t| !t.is_empty()) {
        None => {
            return Reply {
                text: format!(
                    "{NOTHING} `apply: true` writes only a plan you have seen: pass the `plan` token of \
the preview of exactly this text. Here is that preview.\n\n{}",
                    render_preview(&p, vault.root())
                ),
                structured: Some(structured(&p, &plan)),
                is_error: true,
            };
        }
        Some(t) if t != now => {
            return Reply {
                text: format!(
                    "{NOTHING} the vault changed since that preview, so the plan would be different \
now. Check the new preview and apply its token if it is still what you want.\n\n{}",
                    render_preview(&p, vault.root())
                ),
                structured: Some(structured(&p, &plan)),
                is_error: true,
            };
        }
        Some(_) => {}
    }
    let o = match command::run(
        vault,
        text,
        &Context::new(vault.now().date()),
        Some(&plan),
        true,
    ) {
        Ok(o) => o,
        Err(r) => return Reply::err(refusal_text(&r)),
    };
    if let Some(e) = &o.applied.error {
        if o.applied.rolled_back {
            // All or nothing: what was written before the failure was taken back.
            return Reply::err(format!(
                "{NOTHING} {e} (steps written before the failure were taken back)"
            ));
        }
        let mut msg = format!(
            "The write failed and could not be fully taken back: {e}\nStill written (change or remove \
by hand, e.g. with update_task, or ask the user to undo it in the app):"
        );
        for d in &o.applied.left_written {
            msg.push_str(&format!("\n- {d}"));
        }
        msg.push_str("\nEverything else of this plan was taken back or never written.");
        return Reply {
            text: msg,
            structured: Some(
                serde_json::json!({ "written": true, "rolled_back": false, "left_written": o.applied.left_written }),
            ),
            is_error: true,
        };
    }
    let mut out = format!("Written: {}\n", o.preview.reading);
    for w in &o.applied.echo {
        out.push_str(&format!(
            "\n{}:{}\n{}",
            render::abs_path(vault.root(), &w.file),
            w.line,
            fence(std::slice::from_ref(&w.text))
        ));
    }
    Reply {
        text: out,
        structured: Some(serde_json::json!({ "written": true, "lines": o.applied.echo })),
        is_error: false,
    }
}

/// A tool answer: text, an optional structured part, and whether it is an error.
pub struct Reply {
    pub text: String,
    pub structured: Option<serde_json::Value>,
    pub is_error: bool,
}

impl Reply {
    fn err(text: String) -> Reply {
        Reply {
            text,
            structured: None,
            is_error: true,
        }
    }
}

/// `waits_for` / `related` entries into connection lines. `context`: the
/// project the text belongs to (a bare topic slug is looked up there first).
pub fn connection_lines(
    snap: &Snapshot,
    waits_for: &[String],
    related: &[String],
    context: Option<&str>,
    own: Option<&RefTarget>,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (waits, list) in [(true, waits_for), (false, related)] {
        for raw in list {
            let r = raw.trim();
            if r.is_empty() {
                continue;
            }
            let target = target_of(snap, r, context)?;
            if Some(&target) == own {
                return Err(format!("{NOTHING} `{r}` is the item itself."));
            }
            let line = connection_line(waits, &target);
            if !out.contains(&line) {
                out.push(line);
            }
        }
    }
    Ok(out)
}

/// `^id`, `id`, `[[project/topic]]`, `project/topic` or a topic slug/title.
pub fn target_of(snap: &Snapshot, raw: &str, context: Option<&str>) -> Result<RefTarget, String> {
    let r = raw.trim();
    let id = r.trim_start_matches('^');
    if r.starts_with('^') {
        return match snap.task(id) {
            Some(_) if montybrain_core::is_valid_id(id) => {
                Ok(RefTarget::Task { id: id.to_string() })
            }
            _ => Err(format!("{NOTHING} no task with id '{id}'.")),
        };
    }
    // Six lowercase characters may be a task id or a topic slug (`github`,
    // `remont`): the task first, then the topic.
    if montybrain_core::is_valid_id(r) && snap.task(r).is_some() {
        return Ok(RefTarget::Task { id: r.to_string() });
    }
    let body = r.trim_start_matches("[[").trim_end_matches("]]");
    match resolve::resolve_topic_ref(snap, body, context) {
        resolve::Resolved::One(p) => match p.location {
            TaskLocation::Topic { project, topic } => Ok(RefTarget::Topic {
                project,
                slug: topic,
            }),
            _ => Err(format!("{NOTHING} '{r}' is not a topic.")),
        },
        resolve::Resolved::Typo(p, _) => Err(format!(
            "{NOTHING} unknown topic '{r}'; did you mean {}? Use the exact slug.",
            p.text
        )),
        resolve::Resolved::Ambiguous(cs) | resolve::Resolved::Unknown(cs) if !cs.is_empty() => {
            Err(format!(
                "{NOTHING} '{r}' is ambiguous or unknown. Possible: {}.",
                cs.iter()
                    .map(|c| c.insert.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
        _ => Err(format!("{NOTHING} unknown topic '{r}'.")),
    }
}

fn describe(snap: &Snapshot, key: &str) -> String {
    if let Some(rest) = key.strip_prefix("t:") {
        let (p, s) = rest.split_once('/').unwrap_or((rest, ""));
        return match snap.topic(p, s) {
            Some(t) => format!(
                "[[{p}/{s}]] {} ({})",
                t.title,
                render::topic_status(t.status)
            ),
            None => format!("[[{p}/{s}]]"),
        };
    }
    if let Some(id) = key.strip_prefix("k:") {
        return match snap.task(id) {
            Some(t) => format!(
                "^{id} {}{} ({})",
                t.title,
                if t.status == TaskStatus::Done {
                    " [done]"
                } else {
                    ""
                },
                render::location(&t.location)
            ),
            None => format!("^{id}"),
        };
    }
    key.to_string()
}

/// `connections`: both directions and the "waits for" chain, as the map reads them.
pub fn connections(snap: &Snapshot, raw: &str) -> Result<String, String> {
    let target =
        target_of(snap, raw, None).map_err(|e| e.replacen(NOTHING, "", 1).trim().to_string())?;
    let key = target.key();
    let g = build_graph(snap);
    let ns = neighbours(&g, &key);
    let mut out = format!("Connections of {}\n", describe(snap, &key));
    let section = |title: &str, items: Vec<String>, out: &mut String| {
        out.push_str(&format!("\n{title}:"));
        if items.is_empty() {
            out.push_str(" none");
        }
        for i in items {
            out.push_str(&format!("\n- {i}"));
        }
        out.push('\n');
    };
    let pick = |kind: refs::EdgeKind, outgoing: Option<bool>| -> Vec<String> {
        ns.iter()
            .filter(|n| n.edge.kind == kind && outgoing.is_none_or(|o| n.outgoing == o))
            .map(|n| describe(snap, &n.other))
            .collect()
    };
    section(
        "Waits for",
        pick(refs::EdgeKind::Waits, Some(true)),
        &mut out,
    );
    section(
        "Waited for by",
        pick(refs::EdgeKind::Waits, Some(false)),
        &mut out,
    );
    section("Related", pick(refs::EdgeKind::Related, None), &mut out);
    let (chain, cycle) = wait_chain(&g, &key, 4);
    if !chain.is_empty() {
        let names: Vec<String> = chain.iter().map(|k| describe(snap, k)).collect();
        out.push_str(&format!(
            "\nWait chain: {}{}\n",
            names.join(" → "),
            if cycle {
                " → (back to the start: a cycle)"
            } else {
                ""
            }
        ));
    }
    let dangling: Vec<&str> = g
        .dangling
        .iter()
        .filter(|d| d.from == key)
        .map(|d| d.text.as_str())
        .collect();
    if !dangling.is_empty() {
        out.push_str(&format!(
            "\nReferences to things that do not exist: {}\n",
            dangling.join(", ")
        ));
    }
    Ok(out)
}
