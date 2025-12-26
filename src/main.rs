use anyhow::{Context, Result};
use seahorse::{App, Flag, FlagType};
use std::env;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = env::args().collect();
    let app = App::new(env!("CARGO_PKG_NAME"))
        .description("PDFファイルを画像化してから再度PDFに変換するツール")
        .version(env!("CARGO_PKG_VERSION"))
        .usage("pdf_rasterizer <input> <output> [--dpi <value>]")
        .flag(Flag::new("dpi", FlagType::Int).description("ラスタライズ時のDPI（解像度）"))
        .action(|c| {
            let input = PathBuf::from(
                c.args
                    .get(0)
                    .expect("入力PDFファイルのパスを指定してください"),
            );
            let output = PathBuf::from(
                c.args
                    .get(1)
                    .expect("出力PDFファイルのパスを指定してください"),
            );
            let dpi = c.int_flag("dpi").unwrap_or(72) as u32;

            println!("PDFを最適化しています...");
            println!("入力: {}", input.display());
            println!("出力: {}", output.display());
            println!("DPI: {}", dpi);

            if let Err(e) = process_pdf(&input, &output, dpi) {
                eprintln!("エラー: {}", e);
                std::process::exit(1);
            }

            println!("✓ 最適化が完了しました");

            Ok(())
        });

    if let Err(e) = app.run(args) {
        eprintln!("エラー: {}", e);
        std::process::exit(1);
    }
}

fn process_pdf(input_path: &PathBuf, output_path: &PathBuf, dpi: u32) -> Result<()> {
    println!("  hayroを使用してPDFを画像化します...");

    // PDFファイルを読み込み
    let pdf_data = std::fs::read(input_path).with_context(|| {
        format!(
            "PDFファイルの読み込みに失敗しました: {}",
            input_path.display()
        )
    })?;

    let output_data = pdf_rasterizer::rasterize_pdf(pdf_data, dpi)?;

    println!("  PDFを保存しています...");
    std::fs::write(output_path, output_data)
        .context("PDFの保存に失敗しました")?;

    Ok(())
}
