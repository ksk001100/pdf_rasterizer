use anyhow::{Context, Result};
use hayro::{InterpreterSettings, Pdf, RenderSettings};
use seahorse::{App, Flag, FlagType};
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;

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
        });

    app.run(args);
}

fn rasterize_pdf(input_path: &PathBuf, output_path: &PathBuf, dpi: u32) -> Result<()> {
    println!("  hayroを使用してPDFを画像化します...");

    // PDFファイルを読み込み
    let pdf_data = std::fs::read(input_path)
        .with_context(|| format!("PDFファイルの読み込みに失敗しました: {}", input_path.display()))?;

    let pdf = Pdf::new(Arc::new(pdf_data))
        .map_err(|e| anyhow::anyhow!("PDFのパースに失敗しました: {:?}", e))?;

    let page_count = pdf.pages().len();
    println!("  {}ページを処理します", page_count);

    // 一時ディレクトリを作成
    let temp_dir = tempdir().context("一時ディレクトリの作成に失敗しました")?;

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

    let mut image_files = Vec::new();

    // 各ページを画像に変換
    for (page_index, page) in pdf.pages().iter().enumerate() {
        println!("    ページ {}/{} を変換中...", page_index + 1, page_count);

        // ページをレンダリング
        let pixmap = hayro::render(page, &interpreter_settings, &render_settings);

        // PNG形式でエンコード
        let png_data = pixmap.take_png();

        // PNGをデコードしてJPEGとして再エンコード（圧縮のため）
        let image_buffer = image::load_from_memory(&png_data)
            .context("PNG画像のデコードに失敗しました")?
            .to_rgb8();

        // JPEGとして保存
        let image_path = temp_dir
            .path()
            .join(format!("page-{:04}.jpg", page_index + 1));

        // JPEG品質85でエンコード
        let mut jpeg_encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
            std::fs::File::create(&image_path)?,
            85,
        );
        jpeg_encoder
            .encode(
                image_buffer.as_raw(),
                image_buffer.width(),
                image_buffer.height(),
                image::ColorType::Rgb8.into(),
            )
            .context("JPEG画像の保存に失敗しました")?;

        image_files.push(image_path);
    }

    println!("  {}ページの画像を生成しました", image_files.len());
    println!("  画像からPDFを作成中...");

    // 最初の画像を読み込んでPDFのサイズを決定
    let first_image = image::open(&image_files[0]).context("最初の画像の読み込みに失敗しました")?;
    let img_width = first_image.width() as f32;
    let img_height = first_image.height() as f32;

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
    for (page_index, image_path) in image_files.iter().enumerate() {
        println!(
            "    ページ {}/{} を処理中...",
            page_index + 1,
            image_files.len()
        );

        let dynamic_image = image::open(image_path)
            .with_context(|| format!("画像の読み込みに失敗しました: {}", image_path.display()))?;

        let img_w = dynamic_image.width() as f32;
        let img_h = dynamic_image.height() as f32;

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

        // JPEGファイルを直接読み込む
        let jpeg_bytes = std::fs::read(image_path).with_context(|| {
            format!(
                "JPEGファイルの読み込みに失敗しました: {}",
                image_path.display()
            )
        })?;

        // JPEGデータを使ってImageXObjectを作成（DCTEncodeフィルタ付き）
        let image_xobject = printpdf::ImageXObject {
            width: printpdf::Px(img_w as usize),
            height: printpdf::Px(img_h as usize),
            color_space: printpdf::ColorSpace::Rgb,
            bits_per_component: printpdf::ColorBits::Bit8,
            interpolate: true,
            image_data: jpeg_bytes,
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

    println!("  一時ファイルをクリーンアップ中...");
    // tempfileクレートが自動的に一時ディレクトリを削除します

    Ok(())
}
