use wonderland_comment_collector::*;

const HEADER: &str = "// Generated from Rust by examples/generate_bindings.rs. Do not edit.\n";

fn main() {
    let cfg = Config::default();
    let mut out = String::from(HEADER);
    macro_rules! emit { ($($t:ty),*) => { $(out.push_str("export ");out.push_str(&<$t>::decl(&cfg));out.push('\n');)* }; }
    emit!(
        CommentQuery,
        LevelInfo,
        CommentItem,
        CommentArchive,
        ArchiveSummary,
        CollectProgress,
        ExportFormat,
        ExportOutcome
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("ui/src/types.generated.ts");
    if std::env::args().any(|a| a == "--check") {
        assert_eq!(
            std::fs::read_to_string(path).expect("generated file"),
            out,
            "Bindings drift: regenerate bindings"
        );
    } else {
        std::fs::write(path, out).unwrap();
    }
}
