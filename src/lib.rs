use anyhow::{Context, Result};
use hayro::{InterpreterSettings, Pdf, RenderSettings};
use lopdf::ObjectId;
use std::collections::BTreeMap;
use std::sync::Arc;

pub const MIN_DPI: u32 = 72;
pub const MAX_DPI: u32 = 600;
const MAX_SELECTED_PAGES: usize = 100_000;

#[cfg(feature = "wasm")]
mod app;

#[cfg(feature = "wasm")]
pub use app::App;

/// PDFファイルを画像化してから再度PDFに変換する
///
/// `target_pages` が指定された場合、そのページのみをラスタライズします。
/// ページ番号は1から始まります。
pub fn rasterize_pdf(
    pdf_data: Vec<u8>,
    dpi: u32,
    target_pages: Option<Vec<u32>>,
) -> Result<Vec<u8>> {
    validate_dpi(dpi)?;

    #[cfg(feature = "wasm")]
    {
        use gloo_console::log;
        log!("PDFを読み込み中...".to_string());
    }

    // lopdf用のドキュメントを作成（構造変更用）
    let mut doc = lopdf::Document::load_mem(&pdf_data).context("PDFのロードに失敗しました")?;

    if doc.is_encrypted() {
        doc.decrypt(b"")
            .map_err(|e| anyhow::anyhow!("PDFの復号化に失敗しました: {:?}", e))?;
    }

    let page_ids = doc.get_pages();
    let total_pages = page_ids.len();
    if total_pages == 0 {
        return Err(anyhow::anyhow!(
            "ページを検出できませんでした。PDF構造が読み取れないか、暗号化が解除できていません。"
        ));
    }

    // 対象ページを決定
    let target_pages = target_page_numbers(target_pages, &page_ids)?;

    if target_pages.is_empty() {
        return Ok(pdf_data);
    }

    // hayro用のPDFオブジェクトを作成（レンダリング用）
    let pdf = Pdf::new(Arc::new(pdf_data))
        .map_err(|e| anyhow::anyhow!("PDFのパースに失敗しました: {:?}", e))?;

    #[cfg(feature = "wasm")]
    {
        use gloo_console::log;
        log!(format!(
            "{}ページを処理します (全{}ページ)",
            target_pages.len(),
            total_pages
        ));
        log!(format!(
            "選択されたページをJPEG画像に変換中（DPI: {}）...",
            dpi
        ));
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

    // 各ページを画像に変換し、PDFを更新
    // 並列処理のために、まず画像データを生成する
    #[cfg(feature = "cli")]
    let image_data: Result<Vec<_>> = {
        use rayon::prelude::*;
        // move doc inside closure is tricky with rayon if we want to modify it later.
        // So we just render images first.
        target_pages
            .par_iter()
            .map(|&page_num| {
                // hayroは0-indexed
                let page_index = (page_num - 1) as usize;
                if let Some(page) = pdf.pages().get(page_index) {
                    process_page(page, page_num, &interpreter_settings, &render_settings)
                } else {
                    Err(anyhow::anyhow!("ページ {} が見つかりません", page_num))
                }
            })
            .collect()
    };

    #[cfg(not(feature = "cli"))]
    let image_data: Result<Vec<_>> = target_pages
        .iter()
        .map(|&page_num| {
            let page_index = (page_num - 1) as usize;
            if let Some(page) = pdf.pages().get(page_index) {
                process_page(page, page_num, &interpreter_settings, &render_settings)
            } else {
                Err(anyhow::anyhow!("ページ {} が見つかりません", page_num))
            }
        })
        .collect();

    let image_data = image_data?;

    #[cfg(feature = "wasm")]
    {
        use gloo_console::log;
        log!(format!("{}ページの画像を生成しました", image_data.len()));
        log!("PDFを更新中...");
    }

    // 生成した画像でPDFを更新
    for (page_num, jpeg_bytes, img_w, img_h) in image_data {
        let page_id = page_ids
            .get(&page_num)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("ページIDが見つかりません"))?;
        replace_page_content(&mut doc, page_id, jpeg_bytes, img_w, img_h, dpi)?;
    }

    #[cfg(feature = "wasm")]
    {
        use gloo_console::log;
        log!("PDFを保存しています...");
    }

    // PDFをバイト列として保存
    // リニアライズなどを無効化して保存（増分更新ではなく完全書き換え）
    let mut output = Vec::new();
    doc.save_to(&mut output)
        .context("PDFの保存に失敗しました")?;

    Ok(output)
}

fn replace_page_content(
    doc: &mut lopdf::Document,
    page_id: ObjectId,
    jpeg_bytes: Vec<u8>,
    img_w: u32,
    img_h: u32,
    dpi: u32,
) -> Result<()> {
    let (page_width, page_height) = image_size_to_points(img_w, img_h, dpi);

    // 画像XObjectを作成
    let image_stream = lopdf::Stream::new(
        lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"XObject".to_vec())),
            ("Subtype", lopdf::Object::Name(b"Image".to_vec())),
            ("Width", lopdf::Object::Integer(img_w as i64)),
            ("Height", lopdf::Object::Integer(img_h as i64)),
            ("ColorSpace", lopdf::Object::Name(b"DeviceRGB".to_vec())),
            ("BitsPerComponent", lopdf::Object::Integer(8)),
            ("Filter", lopdf::Object::Name(b"DCTDecode".to_vec())),
        ]),
        jpeg_bytes,
    );
    let image_id = doc.add_object(image_stream);

    // XObject名
    let xobject_name = "ImRasterized";

    // コンテンツストリームを作成（画像を配置）
    let content = format!(
        "q\n{} 0 0 {} 0 0 cm\n/{} Do\nQ",
        page_width, page_height, xobject_name
    );

    let content_stream = lopdf::Stream::new(lopdf::Dictionary::new(), content.into_bytes());
    let content_id = doc.add_object(content_stream);

    // ページ辞書を取得して更新
    if let Some(page_obj) = doc.objects.get_mut(&page_id) {
        if let Ok(page_dict) = page_obj.as_dict_mut() {
            // MediaBoxを更新
            page_dict.set(
                "MediaBox",
                vec![0.into(), 0.into(), page_width.into(), page_height.into()],
            );
            page_dict.set(
                "CropBox",
                vec![0.into(), 0.into(), page_width.into(), page_height.into()],
            );
            // hayroが回転・クロップ適用後の画像を返すため、元のページ属性を
            // 残すと出力時に二重適用される。Rotateは継承値も上書きする。
            page_dict.set("Rotate", lopdf::Object::Integer(0));
            page_dict.set("UserUnit", lopdf::Object::Integer(1));
            page_dict.remove(b"BleedBox");
            page_dict.remove(b"TrimBox");
            page_dict.remove(b"ArtBox");

            // Contentsを更新
            page_dict.set("Contents", lopdf::Object::Reference(content_id));

            // Resourcesを更新
            // 既存のResourcesを取得または新規作成
            let resources = if let Ok(res) = page_dict.get_mut(b"Resources") {
                if let Ok(res_dict) = res.as_dict_mut() {
                    res_dict
                } else {
                    // Resourcesが辞書でない場合（参照など）は、簡単のため新規作成してしまう
                    // 複雑なPDFだと問題になる可能性があるが、Contentsを全置換するので
                    // 基本的には新しいResourcesだけで十分なはず
                    page_dict.set("Resources", lopdf::Dictionary::new());
                    page_dict.get_mut(b"Resources")?.as_dict_mut()?
                }
            } else {
                page_dict.set("Resources", lopdf::Dictionary::new());
                page_dict.get_mut(b"Resources")?.as_dict_mut()?
            };

            // 画像XObjectを登録
            let mut xobject_dict = lopdf::Dictionary::new();
            xobject_dict.set(
                xobject_name.as_bytes().to_vec(),
                lopdf::Object::Reference(image_id),
            );
            resources.set("XObject", xobject_dict);

            // Annots（注釈）などを削除（画像化されたので不要なはず）
            page_dict.remove(b"Annots");
            page_dict.remove(b"Group");
        }
    }

    Ok(())
}

fn process_page(
    page: &hayro_syntax::page::Page,
    page_num: u32, // 1-based
    interpreter_settings: &InterpreterSettings,
    render_settings: &RenderSettings,
) -> Result<(u32, Vec<u8>, u32, u32)> {
    // ページをレンダリング
    let pixmap = hayro::render(page, interpreter_settings, render_settings);

    // 幅と高さを取得
    let width = pixmap.width() as u32;
    let height = pixmap.height() as u32;

    // RGBAデータを取得（premultiplied）
    let rgba_data = pixmap.take_u8();

    // RGBAからRGBに変換（alphaチャンネルを除去し、un-premultiply）
    let mut rgb_data = Vec::with_capacity((width * height * 3) as usize);
    for chunk in rgba_data.as_chunks::<4>().0 {
        let r = chunk[0];
        let g = chunk[1];
        let b = chunk[2];
        let a = chunk[3];

        if a == 255 {
            rgb_data.extend_from_slice(&[r, g, b]);
        } else if a == 0 {
            rgb_data.push(0);
            rgb_data.push(0);
            rgb_data.push(0);
        } else {
            rgb_data.push(unpremultiply_channel(r, a));
            rgb_data.push(unpremultiply_channel(g, a));
            rgb_data.push(unpremultiply_channel(b, a));
        }
    }

    // JPEG品質85でメモリ上にエンコード
    let mut jpeg_data = Vec::new();
    let mut jpeg_encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_data, 85);
    jpeg_encoder
        .encode(&rgb_data, width, height, image::ColorType::Rgb8.into())
        .context("JPEG画像のエンコードに失敗しました")?;

    Ok((page_num, jpeg_data, width, height))
}

#[inline]
fn unpremultiply_channel(channel: u8, alpha: u8) -> u8 {
    (((channel as u32 * 255) + (alpha as u32 / 2)) / alpha as u32).min(255) as u8
}

fn image_size_to_points(img_w: u32, img_h: u32, dpi: u32) -> (f32, f32) {
    let dpi = dpi as f32;
    ((img_w as f32 / dpi) * 72.0, (img_h as f32 / dpi) * 72.0)
}

fn validate_dpi(dpi: u32) -> Result<()> {
    if !(MIN_DPI..=MAX_DPI).contains(&dpi) {
        return Err(anyhow::anyhow!(
            "DPIは{}から{}の範囲で指定してください（指定値: {}）",
            MIN_DPI,
            MAX_DPI,
            dpi
        ));
    }
    Ok(())
}

fn target_page_numbers(
    target_pages: Option<Vec<u32>>,
    page_ids: &BTreeMap<u32, ObjectId>,
) -> Result<Vec<u32>> {
    if let Some(pages) = target_pages {
        if let Some(page_num) = pages
            .iter()
            .find(|page_num| !page_ids.contains_key(page_num))
        {
            return Err(anyhow::anyhow!(
                "ページ {} は存在しません（全{}ページ）",
                page_num,
                page_ids.len()
            ));
        }
        Ok(pages)
    } else {
        Ok(page_ids.keys().copied().collect())
    }
}

/// 進捗コールバック付きでPDFを処理する（WASM専用）
#[cfg(feature = "wasm")]
pub async fn rasterize_pdf_with_progress<F>(
    pdf_data: Vec<u8>,
    dpi: u32,
    target_pages: Option<Vec<u32>>,
    progress_callback: F,
) -> Result<Vec<u8>>
where
    F: Fn(String),
{
    use gloo_console::log;
    use gloo_timers::future::TimeoutFuture;

    validate_dpi(dpi)?;

    // UIに「読み込み中」を表示させるために、処理を一度イベントループに戻す
    progress_callback("レンダリング準備中...".to_string());
    TimeoutFuture::new(50).await;

    // 構造変更用 (lopdf) - 早期チェック
    let mut doc = lopdf::Document::load_mem(&pdf_data).context("PDFのロードに失敗しました")?;

    if doc.is_encrypted() {
        doc.decrypt(b"")
            .map_err(|e| anyhow::anyhow!("PDFの復号化に失敗しました: {:?}", e))?;
    }

    let page_ids = doc.get_pages();
    let total_pages = page_ids.len() as u32;
    if total_pages == 0 {
        return Err(anyhow::anyhow!(
            "ページを検出できませんでした。PDF構造が読み取れないか、暗号化が解除できていません。"
        ));
    }

    // 対象ページを決定
    let target_pages = target_page_numbers(target_pages, &page_ids)?;

    if target_pages.is_empty() {
        progress_callback("処理対象のページがありません".to_string());
        return Ok(pdf_data);
    }

    // レンダリング用 (hayro) - 構造チェックが通ってから実行
    let pdf = Pdf::new(Arc::new(pdf_data))
        .map_err(|e| anyhow::anyhow!("PDFのパースに失敗しました: {:?}", e))?;

    log!(format!("{}ページを処理します", target_pages.len()));
    progress_callback(format!(
        "{}ページを対象に処理を開始します",
        target_pages.len()
    ));

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
    let mut images = Vec::new();
    for (i, &page_num) in target_pages.iter().enumerate() {
        progress_callback(format!(
            "ページ {} を画像化中... ({}/{})",
            page_num,
            i + 1,
            target_pages.len()
        ));
        log!(format!("ページ {} を処理中", page_num));

        // hayroは0-indexed
        let page_index = (page_num - 1) as usize;
        if let Some(page) = pdf.pages().get(page_index) {
            let result = process_page(page, page_num, &interpreter_settings, &render_settings)?;
            images.push(result);
        }

        // 各ページ処理後にブラウザに制御を戻す
        TimeoutFuture::new(1).await;
    }

    log!(format!("{}ページの画像を生成しました", images.len()));
    progress_callback("PDFを更新中...".to_string());

    // UIを更新するために少し待機
    TimeoutFuture::new(10).await;

    // 各画像をPDFページとして置換
    for (i, (page_num, jpeg_bytes, img_w, img_h)) in images.into_iter().enumerate() {
        if i % 5 == 0 {
            progress_callback(format!("PDF更新中... ({}/{})", i + 1, target_pages.len()));
            // 5ページごとにUIを更新
            TimeoutFuture::new(1).await;
        }

        let page_id = page_ids
            .get(&page_num)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("ページIDが見つかりません"))?;
        replace_page_content(&mut doc, page_id, jpeg_bytes, img_w, img_h, dpi)?;
    }

    progress_callback("PDFを保存中...".to_string());
    log!("PDFを保存しています...");

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

/// 全ページを強制的にラスタライズして、新しいPDFを作成する（暗号化ファイルなどのフォールバック用）
#[cfg(feature = "wasm")]
pub async fn rasterize_all_new_pdf_with_progress<F>(
    pdf_data: Vec<u8>,
    dpi: u32,
    progress_callback: F,
) -> Result<Vec<u8>>
where
    F: Fn(String),
{
    use gloo_console::log;
    use gloo_timers::future::TimeoutFuture;

    validate_dpi(dpi)?;

    progress_callback("レンダリング準備中...".to_string());
    TimeoutFuture::new(50).await;

    // hayroは暗号化されていてもレンダリング可能（パスワード不要な場合）
    // パスワードが必要な場合はエラーになるが、今回は標準的な閲覧可能PDFを想定
    let pdf = Pdf::new(Arc::new(pdf_data))
        .map_err(|e| anyhow::anyhow!("PDFのパースに失敗しました: {:?}", e))?;

    let total_pages = pdf.pages().len();
    log!(format!("全ページラスタライズモード: {}ページ", total_pages));
    progress_callback(format!("全{}ページを再構築します...", total_pages));

    // UI更新待機
    TimeoutFuture::new(10).await;

    // DPI設定
    let scale = dpi as f32 / 72.0;
    let render_settings = RenderSettings {
        x_scale: scale,
        y_scale: scale,
        width: None,
        height: None,
    };
    let interpreter_settings = InterpreterSettings::default();

    // 全ページを画像化
    let mut images = Vec::new();
    for i in 0..total_pages {
        let page_num = (i + 1) as u32;
        progress_callback(format!(
            "ページ {} を画像化中... ({}/{})",
            page_num, page_num, total_pages
        ));

        if let Some(page) = pdf.pages().get(i) {
            let result = process_page(page, page_num, &interpreter_settings, &render_settings)?;
            images.push(result);
        }
        TimeoutFuture::new(1).await;
    }

    log!("画像を生成完了。新しいPDFを作成します...".to_string());
    progress_callback("新しいPDFを作成中...".to_string());
    TimeoutFuture::new(10).await;

    // 新しいPDFを作成
    // ここではlopdfの機能を使ってゼロから構築する

    let mut doc = lopdf::Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let catalog_id = doc.new_object_id();

    let mut page_ids = Vec::new();

    for (i, (_page_num, jpeg_bytes, img_w, img_h)) in images.into_iter().enumerate() {
        if i % 5 == 0 {
            progress_callback(format!("PDF生成中... ({}/{})", i + 1, total_pages));
            TimeoutFuture::new(1).await;
        }

        let (page_width, page_height) = image_size_to_points(img_w, img_h, dpi);
        let page_id = doc.new_object_id();
        let content_id = doc.new_object_id();
        let image_id = doc.new_object_id();
        let xobject_name = "ImRasterized";

        // 画像ストリーム
        let mut image_dict = lopdf::Dictionary::new();
        image_dict.set("Type", lopdf::Object::Name(b"XObject".to_vec()));
        image_dict.set("Subtype", lopdf::Object::Name(b"Image".to_vec()));
        image_dict.set("Width", lopdf::Object::Integer(img_w as i64));
        image_dict.set("Height", lopdf::Object::Integer(img_h as i64));
        image_dict.set("ColorSpace", lopdf::Object::Name(b"DeviceRGB".to_vec()));
        image_dict.set("BitsPerComponent", lopdf::Object::Integer(8));
        image_dict.set("Filter", lopdf::Object::Name(b"DCTDecode".to_vec()));
        doc.objects.insert(
            image_id,
            lopdf::Object::Stream(lopdf::Stream::new(image_dict, jpeg_bytes)),
        );

        // コンテンツストリーム
        let content = format!(
            "q\n{} 0 0 {} 0 0 cm\n/{} Do\nQ",
            page_width, page_height, xobject_name
        );
        doc.objects.insert(
            content_id,
            lopdf::Object::Stream(lopdf::Stream::new(
                lopdf::Dictionary::new(),
                content.into_bytes(),
            )),
        );

        // Resources
        let mut xobj_dict = lopdf::Dictionary::new();
        xobj_dict.set(
            xobject_name.as_bytes().to_vec(),
            lopdf::Object::Reference(image_id),
        );
        let mut res_dict = lopdf::Dictionary::new();
        res_dict.set("XObject", xobj_dict);

        // ページ辞書
        let mut page_dict = lopdf::Dictionary::new();
        page_dict.set("Type", lopdf::Object::Name(b"Page".to_vec()));
        page_dict.set("Parent", lopdf::Object::Reference(pages_id));
        page_dict.set(
            "MediaBox",
            vec![0.into(), 0.into(), page_width.into(), page_height.into()],
        );
        page_dict.set("Contents", lopdf::Object::Reference(content_id));
        page_dict.set("Resources", res_dict);

        doc.objects
            .insert(page_id, lopdf::Object::Dictionary(page_dict));
        page_ids.push(lopdf::Object::Reference(page_id));
    }

    // Pages (ルート)
    let mut pages_dict = lopdf::Dictionary::new();
    pages_dict.set("Type", lopdf::Object::Name(b"Pages".to_vec()));
    pages_dict.set("Count", lopdf::Object::Integer(page_ids.len() as i64));
    pages_dict.set("Kids", lopdf::Object::Array(page_ids));
    doc.objects
        .insert(pages_id, lopdf::Object::Dictionary(pages_dict));

    // Catalog
    let mut catalog_dict = lopdf::Dictionary::new();
    catalog_dict.set("Type", lopdf::Object::Name(b"Catalog".to_vec()));
    catalog_dict.set("Pages", lopdf::Object::Reference(pages_id));
    doc.objects
        .insert(catalog_id, lopdf::Object::Dictionary(catalog_dict));

    // Trailer
    doc.trailer
        .set("Root", lopdf::Object::Reference(catalog_id));

    progress_callback("完了しました！".to_string());

    let mut output = Vec::new();
    doc.save_to(&mut output)
        .context("PDFの保存に失敗しました")?;

    Ok(output)
}

/// ページ範囲文字列（例: "1, 3-5"）をパースしてページ番号のリストを返す。
/// 空欄は全ページを表す `None`、不正な入力はエラーになる。
pub fn parse_page_range(input: &str) -> Result<Option<Vec<u32>>> {
    if input.trim().is_empty() {
        return Ok(None);
    }
    let mut pages = Vec::new();
    for part in input.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(anyhow::anyhow!("ページ範囲に空の項目があります"));
        }
        if let Some((start, end)) = part.split_once('-') {
            let s = parse_page_number(start.trim())?;
            let e = parse_page_number(end.trim())?;
            if s > e {
                return Err(anyhow::anyhow!(
                    "ページ範囲の開始は終了以下にしてください: {}",
                    part
                ));
            }
            let range_len = (u64::from(e) - u64::from(s) + 1) as usize;
            if pages.len().saturating_add(range_len) > MAX_SELECTED_PAGES {
                return Err(anyhow::anyhow!(
                    "一度に指定できるページ数は{}ページまでです",
                    MAX_SELECTED_PAGES
                ));
            }
            pages.extend(s..=e);
        } else {
            pages.push(parse_page_number(part)?);
            if pages.len() > MAX_SELECTED_PAGES {
                return Err(anyhow::anyhow!(
                    "一度に指定できるページ数は{}ページまでです",
                    MAX_SELECTED_PAGES
                ));
            }
        }
    }
    pages.sort();
    pages.dedup();
    Ok(Some(pages))
}

fn parse_page_number(input: &str) -> Result<u32> {
    let page = input
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("不正なページ番号です: {}", input))?;
    if page == 0 {
        return Err(anyhow::anyhow!("ページ番号は1以上で指定してください"));
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    #[test]
    fn parses_page_ranges() {
        assert_eq!(parse_page_range("").unwrap(), None);
        assert_eq!(
            parse_page_range("5, 1, 3-5").unwrap(),
            Some(vec![1, 3, 4, 5])
        );
    }

    #[test]
    fn rejects_invalid_page_ranges() {
        for input in ["invalid", "0", "3-1", "1,,2", "1-2-3", "1-4294967295"] {
            assert!(parse_page_range(input).is_err(), "accepted {input:?}");
        }
    }

    #[test]
    fn rejects_pages_that_do_not_exist() {
        let page_ids = BTreeMap::from([(1, (1, 0)), (2, (2, 0))]);
        assert!(target_page_numbers(Some(vec![1, 3]), &page_ids).is_err());
    }

    #[test]
    fn rejects_out_of_range_dpi_before_loading_pdf() {
        assert!(rasterize_pdf(Vec::new(), MIN_DPI - 1, None).is_err());
        assert!(rasterize_pdf(Vec::new(), MAX_DPI + 1, None).is_err());
    }

    #[test]
    fn replacement_normalizes_page_geometry() {
        let mut doc = lopdf::Document::with_version("1.7");
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
            "CropBox" => vec![50.into(), 0.into(), 150.into(), 100.into()],
            "BleedBox" => vec![50.into(), 0.into(), 150.into(), 100.into()],
            "Rotate" => 90,
            "UserUnit" => 2,
        });

        replace_page_content(&mut doc, page_id, Vec::new(), 100, 200, 72).unwrap();

        let page = doc.get_object(page_id).unwrap().as_dict().unwrap();
        assert_eq!(page.get(b"Rotate").unwrap().as_i64().unwrap(), 0);
        assert_eq!(page.get(b"UserUnit").unwrap().as_i64().unwrap(), 1);
        assert_eq!(
            page.get(b"MediaBox").unwrap(),
            page.get(b"CropBox").unwrap()
        );
        assert!(!page.has(b"BleedBox"));
    }
}
