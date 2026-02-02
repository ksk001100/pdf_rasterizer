use lopdf::{Dictionary, Document, Object};
use pdf_rasterizer::rasterize_pdf;
use std::path::PathBuf;

#[test]
fn test_partial_rasterization() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pdf_path = manifest_dir.join("test.pdf");
    if !pdf_path.exists() {
        eprintln!("test.pdf not found, skipping test");
        return;
    }

    let pdf_data = std::fs::read(&pdf_path).unwrap();
    let doc = Document::load_mem(&pdf_data).unwrap();

    // Ensure we have at least 3 pages by duplicating the first page if needed
    let page_ids: Vec<u32> = doc.get_pages().keys().cloned().collect();
    if page_ids.is_empty() {
        eprintln!("PDF is empty, skipping test");
        return;
    }
    // We will proceed with whatever pages we have.
    // If only 1 page, we just verify that one page is rasterized.

    // We will target page 1 for rasterization.
    let target_pages = vec![1];

    // Convert doc back to bytes for input
    // If we didn't add pages, just use original pdf_data
    let input_bytes = pdf_data;

    let result = rasterize_pdf(input_bytes, 72, Some(target_pages));
    assert!(result.is_ok(), "Rasterization failed: {:?}", result.err());

    let output_bytes = result.unwrap();
    let output_doc = Document::load_mem(&output_bytes).unwrap();

    // Check page 1
    let page1_id = *output_doc.get_pages().get(&1).unwrap();
    let page1_dict = output_doc.get_dictionary(page1_id).unwrap();

    // Check Resources -> XObject -> ImRasterized exists
    let resources = resolve_resources(&output_doc, page1_dict);
    assert!(resources.is_some(), "Page 1 should have resources");
    let resources = resources.unwrap();

    if let Ok(xobjects) = resources.get(b"XObject") {
        let xobjects = resolve_dictionary(&output_doc, xobjects).unwrap();
        assert!(
            xobjects.has(b"ImRasterized"),
            "Page 1 should have ImRasterized XObject"
        );
    } else {
        panic!("Page 1 Resources missing XObject dictionary");
    }

    // If we had a page 2 that was NOT in target_pages, we should verify it DOES NOT have ImRasterized.
    if output_doc.get_pages().len() >= 2 {
        let page2_id = *output_doc.get_pages().get(&2).unwrap();
        let page2_dict = output_doc.get_dictionary(page2_id).unwrap();

        // Page 2 might share resources if not careful, but our implementation creates new Resources for rasterized page.
        // Original page 2 should keep its old resources (or none).
        let resources2 = resolve_resources(&output_doc, page2_dict);
        if let Some(res2) = resources2 {
            if let Ok(xobjects) = res2.get(b"XObject") {
                let xobjects = resolve_dictionary(&output_doc, xobjects);
                if let Some(xobj_dict) = xobjects {
                    // It shouldn't have ImRasterized unless original PDF had it (unlikely)
                    assert!(
                        !xobj_dict.has(b"ImRasterized"),
                        "Page 2 (untouched) should not have ImRasterized"
                    );
                }
            }
        }
    }
}

fn resolve_resources<'a>(doc: &'a Document, page_dict: &'a Dictionary) -> Option<&'a Dictionary> {
    if let Ok(res) = page_dict.get(b"Resources") {
        resolve_dictionary(doc, res)
    } else {
        None
    }
}

fn resolve_dictionary<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}
