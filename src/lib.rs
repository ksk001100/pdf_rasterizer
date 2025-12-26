use anyhow::{Context, Result};
use hayro::{InterpreterSettings, Pdf, RenderSettings};
use std::sync::Arc;

#[cfg(feature = "wasm")]
mod app;

#[cfg(feature = "wasm")]
pub use app::App;

#[cfg(feature = "wasm")]
use gloo_timers::future::TimeoutFuture;

/// PDFファイルを画像化してから再度PDFに変換する
pub fn rasterize_pdf(pdf_data: Vec<u8>, dpi: u32) -> Result<Vec<u8>> {
    let pdf = Pdf::new(Arc::new(pdf_data))
        .map_err(|e| anyhow::anyhow!("PDFのパースに失敗しました: {:?}", e))?;

    let page_count = pdf.pages().len();

    #[cfg(feature = "wasm")]
    {
        use gloo_console::log;
        log!(format!("{}ページを処理します", page_count));
        log!(format!("PDFをJPEG画像に変換中（DPI: {}）...", dpi));
    }

    // DPIからスケールを計算（72 DPI = 1.0スケール）
    let scale = dpi as f32 / 72.0;

    let render_settings = RenderSettings {
        x_scale: scale,
        y_scale: scale,
        width: None,  // 自動計算
        height: None, // 自動計算
    };

    let interpreter_settings = InterpreterSettings::default();

    // 各ページをメモリ上で画像に変換
    #[cfg(feature = "cli")]
    let image_data: Result<Vec<_>> = {
        use rayon::prelude::*;
        pdf.pages()
            .par_iter()
            .enumerate()
            .map(|(page_index, page)| process_page(page, page_index, &interpreter_settings, &render_settings))
            .collect()
    };

    #[cfg(not(feature = "cli"))]
    let image_data: Result<Vec<_>> = pdf
        .pages()
        .iter()
        .enumerate()
        .map(|(page_index, page)| process_page(page, page_index, &interpreter_settings, &render_settings))
        .collect();

    let mut image_data = image_data?;
    // ページ順にソート
    image_data.sort_by_key(|(idx, _, _, _)| *idx);

    #[cfg(feature = "wasm")]
    {
        use gloo_console::log;
        log!(format!("{}ページの画像を生成しました", image_data.len()));
        log!("画像からPDFを作成中...");
    }

    // lopdfを使ってPDF ドキュメントを作成
    let mut doc = lopdf::Document::with_version("1.5");

    // 各画像をPDFページとして追加
    for (page_num, (_, jpeg_bytes, img_w, img_h)) in image_data.iter().enumerate() {
        let img_w = *img_w as f32;
        let img_h = *img_h as f32;

        let page_width = (img_w / dpi as f32) * 72.0;  // ポイント単位に変換
        let page_height = (img_h / dpi as f32) * 72.0;

        // ページIDを作成
        let page_id = doc.new_object_id();

        // 画像XObjectを作成
        let image_id = doc.add_object(lopdf::Stream::new(
            lopdf::Dictionary::from_iter(vec![
                ("Type", lopdf::Object::Name(b"XObject".to_vec())),
                ("Subtype", lopdf::Object::Name(b"Image".to_vec())),
                ("Width", lopdf::Object::Integer(img_w as i64)),
                ("Height", lopdf::Object::Integer(img_h as i64)),
                ("ColorSpace", lopdf::Object::Name(b"DeviceRGB".to_vec())),
                ("BitsPerComponent", lopdf::Object::Integer(8)),
                ("Filter", lopdf::Object::Name(b"DCTDecode".to_vec())),
            ]),
            jpeg_bytes.clone(),
        ));

        // コンテンツストリームを作成（画像を配置）
        let content = format!(
            "q\n{} 0 0 {} 0 0 cm\n/Im{} Do\nQ",
            page_width, page_height, page_num
        );

        let content_id = doc.add_object(lopdf::Stream::new(
            lopdf::Dictionary::new(),
            content.into_bytes(),
        ));

        // XObject辞書を作成
        let mut xobject_dict = lopdf::Dictionary::new();
        xobject_dict.set(
            format!("Im{}", page_num).into_bytes(),
            lopdf::Object::Reference(image_id),
        );

        // Resourcesディクショナリを作成
        let mut resources_dict = lopdf::Dictionary::new();
        resources_dict.set("XObject", xobject_dict);
        let resources_id = doc.add_object(resources_dict);

        // ページオブジェクトを作成
        let page_dict = lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"Page".to_vec())),
            (
                "MediaBox",
                vec![0.into(), 0.into(), page_width.into(), page_height.into()].into(),
            ),
            ("Contents", lopdf::Object::Reference(content_id)),
            ("Resources", lopdf::Object::Reference(resources_id)),
        ]);

        doc.objects.insert(page_id, lopdf::Object::Dictionary(page_dict));
    }

    // すべてのページを収集
    let page_ids: Vec<_> = image_data.iter().enumerate().map(|(i, _)| {
        // ページIDは追加した順序で計算される
        (1 + i * 4) as u32 // 各ページにつき4つのオブジェクトが作成されている
    }).collect();

    // Pagesオブジェクトを作成
    let pages_id = doc.new_object_id();
    doc.objects.insert(
        pages_id,
        lopdf::Dictionary::from_iter(vec![
            ("Type", "Pages".into()),
            ("Count", (image_data.len() as i64).into()),
            (
                "Kids",
                lopdf::Object::Array(
                    page_ids.iter().map(|&id| lopdf::Object::Reference((id, 0))).collect()
                ),
            ),
        ])
        .into(),
    );

    // すべてのページにParentを設定
    for &page_id_val in &page_ids {
        if let Some(page_obj) = doc.objects.get_mut(&(page_id_val, 0)) {
            if let Ok(page_dict) = page_obj.as_dict_mut() {
                page_dict.set("Parent", lopdf::Object::Reference(pages_id));
            }
        }
    }

    // Catalogオブジェクトを作成
    let catalog_id = doc.new_object_id();
    doc.objects.insert(
        catalog_id,
        lopdf::Dictionary::from_iter(vec![
            ("Type", "Catalog".into()),
            ("Pages", lopdf::Object::Reference(pages_id)),
        ])
        .into(),
    );

    // Trailerを設定
    doc.trailer.set("Root", lopdf::Object::Reference(catalog_id));

    #[cfg(feature = "wasm")]
    {
        use gloo_console::log;
        log!("PDFを生成しています...");
    }

    // PDFをバイト列として保存
    let mut output = Vec::new();
    doc.save_to(&mut output)
        .context("PDFの保存に失敗しました")?;

    Ok(output)
}

fn process_page(
    page: &hayro_syntax::page::Page,
    page_index: usize,
    interpreter_settings: &InterpreterSettings,
    render_settings: &RenderSettings,
) -> Result<(usize, Vec<u8>, u32, u32)> {
    // ページをレンダリング
    let pixmap = hayro::render(page, interpreter_settings, render_settings);

    // 幅と高さを取得
    let width = pixmap.width() as u32;
    let height = pixmap.height() as u32;

    // RGBAデータを取得（premultiplied）
    let rgba_data = pixmap.take_u8();

    // RGBAからRGBに変換（alphaチャンネルを除去し、un-premultiply）
    let mut rgb_data = Vec::with_capacity((width * height * 3) as usize);
    for chunk in rgba_data.chunks_exact(4) {
        let r = chunk[0];
        let g = chunk[1];
        let b = chunk[2];
        let a = chunk[3];

        // Un-premultiply（alphaが0の場合は除算しない）
        if a > 0 {
            let factor = 255.0 / a as f32;
            rgb_data.push((r as f32 * factor).min(255.0) as u8);
            rgb_data.push((g as f32 * factor).min(255.0) as u8);
            rgb_data.push((b as f32 * factor).min(255.0) as u8);
        } else {
            rgb_data.push(0);
            rgb_data.push(0);
            rgb_data.push(0);
        }
    }

    // RGB ImageBufferを作成
    let image_buffer = image::RgbImage::from_vec(width, height, rgb_data)
        .context("RGB画像バッファの作成に失敗しました")?;

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
}

/// 進捗コールバック付きでPDFを処理する（WASM専用）
#[cfg(feature = "wasm")]
pub async fn rasterize_pdf_with_progress<F>(
    pdf_data: Vec<u8>,
    dpi: u32,
    progress_callback: F,
) -> Result<Vec<u8>>
where
    F: Fn(String),
{
    use gloo_console::log;

    let pdf = Pdf::new(Arc::new(pdf_data))
        .map_err(|e| anyhow::anyhow!("PDFのパースに失敗しました: {:?}", e))?;

    let page_count = pdf.pages().len();
    log!(format!("{}ページを処理します", page_count));
    progress_callback(format!("{}ページを読み込みました", page_count));

    // UIを更新するために少し待機
    TimeoutFuture::new(10).await;

    // DPIからスケールを計算（72 DPI = 1.0スケール）
    let scale = dpi as f32 / 72.0;

    let render_settings = RenderSettings {
        x_scale: scale,
        y_scale: scale,
        width: None,
        height: None,
    };

    let interpreter_settings = InterpreterSettings::default();

    // 各ページを順番に処理（非同期）
    let mut image_data = Vec::new();
    for (page_index, page) in pdf.pages().iter().enumerate() {
        progress_callback(format!(
            "ページ {}/{} を画像化中...",
            page_index + 1,
            page_count
        ));
        log!(format!("ページ {}/{} を処理中", page_index + 1, page_count));

        let result = process_page(page, page_index, &interpreter_settings, &render_settings)?;
        image_data.push(result);

        // 各ページ処理後にブラウザに制御を戻す
        TimeoutFuture::new(1).await;
    }

    log!(format!("{}ページの画像を生成しました", image_data.len()));
    progress_callback("PDFを作成中...".to_string());

    // UIを更新するために少し待機
    TimeoutFuture::new(10).await;

    // lopdfを使ってPDF ドキュメントを作成
    let mut doc = lopdf::Document::with_version("1.5");

    // 各画像をPDFページとして追加
    for (page_num, (_, jpeg_bytes, img_w, img_h)) in image_data.iter().enumerate() {
        if page_num % 5 == 0 {
            progress_callback(format!(
                "PDF作成中... ({}/{})",
                page_num + 1,
                image_data.len()
            ));
            // 5ページごとにUIを更新
            TimeoutFuture::new(1).await;
        }

        let img_w = *img_w as f32;
        let img_h = *img_h as f32;

        let page_width = (img_w / dpi as f32) * 72.0;
        let page_height = (img_h / dpi as f32) * 72.0;

        // ページIDを作成
        let page_id = doc.new_object_id();

        // 画像XObjectを作成
        let image_id = doc.add_object(lopdf::Stream::new(
            lopdf::Dictionary::from_iter(vec![
                ("Type", lopdf::Object::Name(b"XObject".to_vec())),
                ("Subtype", lopdf::Object::Name(b"Image".to_vec())),
                ("Width", lopdf::Object::Integer(img_w as i64)),
                ("Height", lopdf::Object::Integer(img_h as i64)),
                ("ColorSpace", lopdf::Object::Name(b"DeviceRGB".to_vec())),
                ("BitsPerComponent", lopdf::Object::Integer(8)),
                ("Filter", lopdf::Object::Name(b"DCTDecode".to_vec())),
            ]),
            jpeg_bytes.clone(),
        ));

        // コンテンツストリームを作成（画像を配置）
        let content = format!(
            "q\n{} 0 0 {} 0 0 cm\n/Im{} Do\nQ",
            page_width, page_height, page_num
        );

        let content_id = doc.add_object(lopdf::Stream::new(
            lopdf::Dictionary::new(),
            content.into_bytes(),
        ));

        // XObject辞書を作成
        let mut xobject_dict = lopdf::Dictionary::new();
        xobject_dict.set(
            format!("Im{}", page_num).into_bytes(),
            lopdf::Object::Reference(image_id),
        );

        // Resourcesディクショナリを作成
        let mut resources_dict = lopdf::Dictionary::new();
        resources_dict.set("XObject", xobject_dict);
        let resources_id = doc.add_object(resources_dict);

        // ページオブジェクトを作成
        let page_dict = lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"Page".to_vec())),
            (
                "MediaBox",
                vec![0.into(), 0.into(), page_width.into(), page_height.into()].into(),
            ),
            ("Contents", lopdf::Object::Reference(content_id)),
            ("Resources", lopdf::Object::Reference(resources_id)),
        ]);

        doc.objects.insert(page_id, lopdf::Object::Dictionary(page_dict));
    }

    // すべてのページを収集
    let page_ids: Vec<_> = image_data
        .iter()
        .enumerate()
        .map(|(i, _)| (1 + i * 4) as u32)
        .collect();

    // Pagesオブジェクトを作成
    let pages_id = doc.new_object_id();
    doc.objects.insert(
        pages_id,
        lopdf::Dictionary::from_iter(vec![
            ("Type", "Pages".into()),
            ("Count", (image_data.len() as i64).into()),
            (
                "Kids",
                lopdf::Object::Array(
                    page_ids
                        .iter()
                        .map(|&id| lopdf::Object::Reference((id, 0)))
                        .collect(),
                ),
            ),
        ])
        .into(),
    );

    // すべてのページにParentを設定
    for &page_id_val in &page_ids {
        if let Some(page_obj) = doc.objects.get_mut(&(page_id_val, 0)) {
            if let Ok(page_dict) = page_obj.as_dict_mut() {
                page_dict.set("Parent", lopdf::Object::Reference(pages_id));
            }
        }
    }

    // Catalogオブジェクトを作成
    let catalog_id = doc.new_object_id();
    doc.objects.insert(
        catalog_id,
        lopdf::Dictionary::from_iter(vec![
            ("Type", "Catalog".into()),
            ("Pages", lopdf::Object::Reference(pages_id)),
        ])
        .into(),
    );

    // Trailerを設定
    doc.trailer.set("Root", lopdf::Object::Reference(catalog_id));

    progress_callback("PDFを保存中...".to_string());
    log!("PDFを生成しています...");

    // UIを更新するために少し待機
    TimeoutFuture::new(10).await;

    // PDFをバイト列として保存
    let mut output = Vec::new();
    doc.save_to(&mut output)
        .context("PDFの保存に失敗しました")?;

    log!("完了しました");
    progress_callback("完了しました！".to_string());

    Ok(output)
}
