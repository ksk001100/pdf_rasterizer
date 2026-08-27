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
        .flag(Flag::new("pages", FlagType::String).description("対象ページ範囲（例: 1,3-5）"))
        .action(|c| {
            if c.args.len() != 2 {
                return Err("入力PDFと出力PDFのパスを1つずつ指定してください".into());
            }
            let input = PathBuf::from(&c.args[0]);
            let output = PathBuf::from(&c.args[1]);
            let dpi_value = c
                .int_flag("dpi")
                .unwrap_or(pdf_rasterizer::MIN_DPI as isize);
            let dpi = u32::try_from(dpi_value).map_err(|_| {
                format!(
                    "DPIは{}から{}の範囲で指定してください",
                    pdf_rasterizer::MIN_DPI,
                    pdf_rasterizer::MAX_DPI
                )
            })?;
            if !(pdf_rasterizer::MIN_DPI..=pdf_rasterizer::MAX_DPI).contains(&dpi) {
                return Err(format!(
                    "DPIは{}から{}の範囲で指定してください（指定値: {}）",
                    pdf_rasterizer::MIN_DPI,
                    pdf_rasterizer::MAX_DPI,
                    dpi
                )
                .into());
            }
            let pages_str = c.string_flag("pages").unwrap_or_default();
            let target_pages = pdf_rasterizer::parse_page_range(&pages_str)?;

            println!("PDFを最適化しています...");
            println!("入力: {}", input.display());
            println!("出力: {}", output.display());
            println!("DPI: {}", dpi);
            if let Some(pages) = &target_pages {
                println!("対象ページ: {:?}", pages);
            } else {
                println!("対象ページ: 全て");
            }

            if let Err(e) = process_pdf(&input, &output, dpi, target_pages) {
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

fn process_pdf(
    input_path: &PathBuf,
    output_path: &PathBuf,
    dpi: u32,
    target_pages: Option<Vec<u32>>,
) -> Result<()> {
    println!("  hayroを使用してPDFを画像化します...");

    // PDFファイルを読み込み
    let pdf_data = std::fs::read(input_path).with_context(|| {
        format!(
            "PDFファイルの読み込みに失敗しました: {}",
            input_path.display()
        )
    })?;

    let output_data = pdf_rasterizer::rasterize_pdf(pdf_data, dpi, target_pages)?;

    println!("  PDFを保存しています...");
    std::fs::write(output_path, output_data).context("PDFの保存に失敗しました")?;

    Ok(())
}
