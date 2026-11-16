//! The skills library (issue #141, R25): add from text or files, list, view,
//! delete (refused while an agent lists it), and agents refuse unknown skills.

use std::sync::Arc;

use xherd_core::agent_def::AgentDef;
use xherd_core::paths::Paths;
use xherd_daemon::skills::{NewSkill, NewSkillFile};
use xherd_daemon::*;

struct T {
    home: tempfile::TempDir,
    d: Daemon,
}

async fn setup() -> T {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    xherd_core::config::Config {
        workspace: Some(home.path().join("ws").to_string_lossy().into_owned()),
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let d = Daemon::start(paths, o).await.unwrap();
    T { home, d }
}

fn text(name: &str, description: &str) -> NewSkill {
    NewSkill {
        name: name.into(),
        text: Some(format!(
            "---\nname: {name}\ndescription: {description}\n---\nDo it.\n"
        )),
        files: None,
        replace: false,
    }
}

fn agent(name: &str, skills: &[&str]) -> AgentDef {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "runtime": "claude-code",
        "skills": skills,
    }))
    .unwrap()
}

#[tokio::test]
async fn add_from_text_list_and_view() {
    let t = setup().await;
    assert!(t.d.skills().await.unwrap().is_empty());
    let s =
        t.d.create_skill(text("lint-fix", "Fixes lint"))
            .await
            .unwrap();
    assert_eq!(s.info.description, "Fixes lint");
    assert_eq!(
        (s.info.files, s.file_list[0].path.as_str()),
        (1, "SKILL.md")
    );
    assert!(s.text.contains("Do it."));
    assert_eq!(s.info.sha256.len(), 64);
    let all = t.d.skills().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "lint-fix");
    assert!(all[0].used_by.is_empty());
    assert!(t.home.path().join("skills/lint-fix/SKILL.md").is_file());
    assert_eq!(
        t.d.skill_detail("nope").await.unwrap_err().code,
        "no_such_skill"
    );
    assert_eq!(
        t.d.skill_detail("../x").await.unwrap_err().code,
        "no_such_skill"
    );
}

#[tokio::test]
async fn add_from_files_and_refuse_bad_ones() {
    let t = setup().await;
    let f = |p: &str, text: Option<&str>, b64: Option<&str>| NewSkillFile {
        path: p.into(),
        text: text.map(Into::into),
        base64: b64.map(Into::into),
    };
    let main = "---\nname: pack\ndescription: With files\n---\n";
    let s =
        t.d.create_skill(NewSkill {
            name: "pack".into(),
            text: None,
            files: Some(vec![
                f("SKILL.md", Some(main), None),
                f("ref/a.md", Some("A"), None),
                f("bin/x", None, Some("eA==")),
            ]),
            replace: false,
        })
        .await
        .unwrap();
    assert_eq!(s.info.files, 3);
    assert_eq!(
        std::fs::read(t.home.path().join("skills/pack/bin/x")).unwrap(),
        b"x"
    );

    let code = |r: Result<_, ActionError>| match r {
        Ok(_) => "ok",
        Err(e) => e.code,
    };
    let bad = |name: &str, body: &str| NewSkill {
        name: name.into(),
        text: Some(body.into()),
        files: None,
        replace: false,
    };
    // no frontmatter, a different name inside, no description, a bad name
    assert_eq!(code(t.d.create_skill(bad("a", "Body")).await), "bad_skill");
    assert_eq!(
        code(
            t.d.create_skill(bad("a", "---\nname: b\ndescription: x\n---\n"))
                .await
        ),
        "bad_skill"
    );
    assert_eq!(
        code(t.d.create_skill(bad("a", "---\nname: a\n---\n")).await),
        "bad_skill"
    );
    assert_eq!(
        code(t.d.create_skill(bad("Bad Name", "x")).await),
        "bad_name"
    );
    // hidden file, parent path, missing SKILL.md, too large
    for files in [
        vec![f("SKILL.md", Some(main), None), f(".env", Some("x"), None)],
        vec![f("SKILL.md", Some(main), None), f("../x", Some("x"), None)],
        vec![f("other.md", Some("x"), None)],
    ] {
        let r =
            t.d.create_skill(NewSkill {
                name: "pack2".into(),
                text: None,
                files: Some(files),
                replace: false,
            })
            .await;
        assert_eq!(code(r), "bad_skill");
    }
    let big = NewSkill {
        name: "big".into(),
        text: Some(format!(
            "---\nname: big\ndescription: x\n---\n{}",
            "x".repeat(2 * 1024 * 1024)
        )),
        files: None,
        replace: false,
    };
    assert_eq!(code(t.d.create_skill(big).await), "too_large");
    // nothing half written
    let left: Vec<_> = std::fs::read_dir(t.home.path().join("skills"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, vec!["pack".to_string()]);
}

#[tokio::test]
async fn an_existing_name_needs_replace() {
    let t = setup().await;
    t.d.create_skill(text("s", "one")).await.unwrap();
    assert_eq!(
        t.d.create_skill(text("s", "two")).await.unwrap_err().code,
        "name_taken"
    );
    let mut again = text("s", "two");
    again.replace = true;
    assert_eq!(
        t.d.create_skill(again).await.unwrap().info.description,
        "two"
    );
}

#[tokio::test]
async fn symlinks_and_hidden_files_in_the_folder_are_not_listed() {
    let t = setup().await;
    t.d.create_skill(text("ok", "fine")).await.unwrap();
    let dir = t.home.path().join("skills/linked");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: linked\ndescription: x\n---\n",
    )
    .unwrap();
    std::os::unix::fs::symlink("/etc/passwd", dir.join("p")).unwrap();
    let names: Vec<_> =
        t.d.skills()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
    assert_eq!(names, vec!["ok".to_string()]);
    assert_eq!(t.d.skill("linked").unwrap_err().code, "bad_skill");
}

#[tokio::test]
async fn agents_refuse_unknown_skills_and_block_delete() {
    let t = setup().await;
    assert_eq!(
        t.d.create_agent(agent("a1", &["lint-fix"]))
            .await
            .unwrap_err()
            .code,
        "unknown_skill"
    );
    t.d.create_skill(text("lint-fix", "Fixes lint"))
        .await
        .unwrap();
    t.d.create_agent(agent("a1", &["lint-fix"])).await.unwrap();
    assert_eq!(
        t.d.skills().await.unwrap()[0].used_by,
        vec!["a1".to_string()]
    );
    let e = t.d.delete_skill("lint-fix").await.unwrap_err();
    assert_eq!(e.code, "skill_in_use");
    assert!(e.message.contains("a1"));
    // an edit that drops the skill frees it
    t.d.update_agent("a1", agent("a1", &[])).await.unwrap();
    t.d.delete_skill("lint-fix").await.unwrap();
    assert!(!t.home.path().join("skills/lint-fix").exists());
    assert_eq!(
        t.d.delete_skill("lint-fix").await.unwrap_err().code,
        "no_such_skill"
    );
}
