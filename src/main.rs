use anyhow::{Context, Result};
use hayro::{InterpreterSettings, Pdf, RenderSettings};
use rayon::prelude::*;
use seahorse::{App, Flag, FlagType};
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

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

            if let Err(e) = rasterize_pdf(&input, &output, dpi) {
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

fn rasterize_pdf(input_path: &PathBuf, output_path: &PathBuf, dpi: u32) -> Result<()> {
    println!("  hayroを使用してPDFを画像化します...");

    // PDFファイルを読み込み
    let pdf_data = std::fs::read(input_path).with_context(|| {
        format!(
            "PDFファイルの読み込みに失敗しました: {}",
            input_path.display()
        )
    })?;

    let pdf = Pdf::new(Arc::new(pdf_data))
        .map_err(|e| anyhow::anyhow!("PDFのパースに失敗しました: {:?}", e))?;

    let page_count = pdf.pages().len();
    println!("  {}ページを処理します", page_count);
    println!("  PDFをJPEG画像に変換中（DPI: {}）...", dpi);

    // DPIからスケールを計算（72 DPI = 1.0スケール）
    let scale = dpi as f32 / 72.0;

    let render_settings = RenderSettings {
        x_scale: scale,
        y_scale: scale,
        width: None,  // 自動計算
        height: None, // 自動計算
    };

    let interpreter_settings = InterpreterSettings::default();

    // 各ページを並列でメモリ上で画像に変換
    let image_data: Result<Vec<_>> = pdf
        .pages()
        .par_iter()
        .enumerate()
        .map(|(page_index, page)| {
            // ページをレンダリング
            let pixmap = hayro::render(page, &interpreter_settings, &render_settings);

            // PNG形式でエンコード
            let png_data = pixmap.take_png();

            // PNGをデコードしてJPEGとして再エンコード（圧縮のため）
            let image_buffer = image::load_from_memory(&png_data)
                .context("PNG画像のデコードに失敗しました")?
                .to_rgb8();

            let width = image_buffer.width();
            let height = image_buffer.height();

            // JPEG品質85でメモリ上にエンコード
            let mut jpeg_data = Vec::new();
            let mut jpeg_encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_data, 85);
            jpeg_encoder
                .encode(
                    image_buffer.as_raw(),
                    width,
                    height,
                    image::ColorType::Rgb8.into(),
                )
                .context("JPEG画像のエンコードに失敗しました")?;

            Ok((page_index, jpeg_data, width, height))
        })
        .collect();

    let mut image_data = image_data?;
    // ページ順にソート
    image_data.sort_by_key(|(idx, _, _, _)| *idx);

    println!("  {}ページの画像を生成しました", image_data.len());
    println!("  画像からPDFを作成中...");

    // 最初の画像のサイズを取得してPDFのサイズを決定
    let (_, _, first_width, first_height) = &image_data[0];
    let img_width = *first_width as f32;
    let img_height = *first_height as f32;

    // PDFのサイズを計算（DPIから）
    let width_mm = (img_width / dpi as f32) * 25.4;
    let height_mm = (img_height / dpi as f32) * 25.4;

    let (doc_pdf, page1_idx, layer1) = printpdf::PdfDocument::new(
        "Optimized PDF",
        printpdf::Mm(width_mm),
        printpdf::Mm(height_mm),
        "Layer 1",
    );
    let mut current_layer = doc_pdf.get_page(page1_idx).get_layer(layer1);

    // 各画像をPDFに追加
    for (page_index, (_, jpeg_bytes, img_w, img_h)) in image_data.iter().enumerate() {
        let img_w = *img_w as f32;
        let img_h = *img_h as f32;

        // 新しいページが必要な場合は追加（最初のページ以外）
        if page_index > 0 {
            let page_width_mm = (img_w / dpi as f32) * 25.4;
            let page_height_mm = (img_h / dpi as f32) * 25.4;
            let (page_idx, layer_idx) = doc_pdf.add_page(
                printpdf::Mm(page_width_mm),
                printpdf::Mm(page_height_mm),
                "Layer 1",
            );
            current_layer = doc_pdf.get_page(page_idx).get_layer(layer_idx);
        }

        // JPEGデータを使ってImageXObjectを作成（DCTEncodeフィルタ付き）
        let image_xobject = printpdf::ImageXObject {
            width: printpdf::Px(img_w as usize),
            height: printpdf::Px(img_h as usize),
            color_space: printpdf::ColorSpace::Rgb,
            bits_per_component: printpdf::ColorBits::Bit8,
            interpolate: true,
            image_data: jpeg_bytes.clone(),
            image_filter: Some(printpdf::ImageFilter::DCT), // JPEG圧縮を保持
            clipping_bbox: None,
            smask: None,
        };

        let img = printpdf::Image::from(image_xobject);

        // 画像をページ全体に配置
        // DPIを設定することで、printpdfが自動的に正しいサイズで配置する
        img.add_to_layer(
            current_layer.clone(),
            printpdf::ImageTransform {
                dpi: Some(dpi as f32),
                ..Default::default()
            },
        );
    }

    println!("  PDFを保存しています...");
    doc_pdf
        .save(&mut std::io::BufWriter::new(std::fs::File::create(
            output_path,
        )?))
        .context("PDFの保存に失敗しました")?;

    Ok(())
}
