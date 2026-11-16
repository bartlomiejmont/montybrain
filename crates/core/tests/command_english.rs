//! English input uses the same parser and Markdown format as Polish input.
use montybrain_core::{Priority, TaskLocation, Vault, command};

#[test]
fn english_capture_aliases_write_the_same_canonical_data() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(dir.path().join("vault")).unwrap();
    let today = "2026-11-16".parse().unwrap();
    let context = command::Context::new(today);
    let output = command::run(&vault, "Call Żaneta @tomorrow !high", &context, None, true).unwrap();
    assert!(output.applied.steps.iter().all(|step| step.ok));
    let snapshot = vault.load().unwrap();
    let task = &snapshot.tasks[0];
    assert_eq!(task.title, "Call Żaneta");
    assert_eq!(task.priority, Priority::High);
    assert_eq!(task.due.unwrap().to_string(), "2026-11-17");
    assert_eq!(task.location, TaskLocation::Inbox);
}

#[test]
fn english_dates_commands_and_priorities_match_polish_aliases() {
    use command::dates::parse_due_word;
    let today = "2026-11-16".parse().unwrap();
    for (en, pl) in [("today", "dziś"), ("tomorrow", "jutro"), ("fri", "pt"), ("sunday", "nd"), ("+2w", "+2t")] {
        assert_eq!(parse_due_word(en).unwrap().resolve(today), parse_due_word(pl).unwrap().resolve(today));
    }
    for (en, pl) in [("/topic Goal > project", "/temat Goal > project"), ("^abc123 /wait ^def456", "^abc123 /czeka ^def456"), ("^abc123 /link ^def456", "^abc123 /powiaz ^def456"), ("Call !low", "Call !niski")] {
        assert!(command::parse(en).problems.is_empty(), "{en}");
        assert!(command::parse(pl).problems.is_empty(), "{pl}");
    }
}
