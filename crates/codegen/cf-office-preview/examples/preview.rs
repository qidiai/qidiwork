//! 手动验证: `cargo run -p cf-office-preview --example preview -- <文件路径>`

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: preview <file-path>");
        std::process::exit(2);
    });
    let content = cf_office_preview::preview_path(&path);
    println!("{}", cf_office_preview::render_to_text(&content));
}
