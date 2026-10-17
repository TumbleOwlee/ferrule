mod golden;

use golden::{Run, fixtures};

fn doc() -> String {
    fixtures().join("md/doc.md").display().to_string()
}

fn text(args: &[&str]) -> Run {
    let mut all = args.to_vec();
    all.extend(["--format", "text"]);
    Run::new(all).env("NO_COLOR", "1").env("COLUMNS", "80")
}

#[test]
fn it_piped_output_stays_markdown() {
    let piped = Run::new(["spec", "section", "Alpha", &doc()]).text();
    let md = Run::new(["spec", "section", "Alpha", &doc(), "--format", "md"]).text();
    assert_eq!(piped, md);
    assert!(piped.starts_with("## Alpha\n"), "{piped}");
}

#[test]
fn it_format_text_draws_headings_as_a_tree() {
    assert_eq!(
        text(&["spec", "sections", &doc()]).text(),
        "Title\n├─ Alpha\n│  └─ Alpha child\n└─ Beta\nexit=0"
    );
}

#[test]
fn it_format_text_indents_a_section_by_its_headings() {
    assert_eq!(
        text(&["spec", "section", "Alpha", "beta", &doc()]).text(),
        "Alpha\n  Alpha body.\n\n    # not a heading\n\n  Alpha child\n    Child body.\n\nBeta\n  Beta body.\nexit=0"
    );
}

#[test]
fn it_format_text_colours_unless_no_color() {
    let coloured = Run::new(["spec", "sections", &doc(), "--format", "text"])
        .env_remove("NO_COLOR")
        .text();
    assert!(coloured.contains("\x1b[1mTitle\x1b[0m"), "{coloured}");
    assert!(!text(&["spec", "sections", &doc()]).text().contains('\x1b'));
}

#[test]
fn it_format_text_lays_out_a_card() {
    let dir = tempfile::tempdir().unwrap();
    let board = fixtures().join("board");
    let run = |args: &[&str]| {
        text(args)
            .env("CLAUDE_PROJECT_DIR", dir.path())
            .cwd(dir.path())
    };
    let imported = run(&["card", "import", board.to_str().unwrap()]).text();
    assert!(imported.starts_with("import=ok"), "{imported}");

    let shown = run(&["card", "show", "demo"]).text();
    assert!(!shown.contains("---"), "{shown}");
    let first = shown.lines().next().unwrap();
    assert!(
        first.starts_with("id ") && first.ends_with(" demo"),
        "{shown}"
    );
    assert!(shown.lines().any(|l| l.starts_with("state ")), "{shown}");

    let board_md = Run::new(["card", "board"])
        .env("CLAUDE_PROJECT_DIR", dir.path())
        .cwd(dir.path())
        .text();
    assert_eq!(run(&["card", "board"]).text(), board_md);
}
