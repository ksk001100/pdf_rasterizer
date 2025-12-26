use gloo_console::log;
use gloo_file::callbacks::FileReader;
use gloo_file::File;
use wasm_bindgen::JsCast;
use web_sys::{Event, HtmlInputElement};
use yew::prelude::*;

pub enum Msg {
    FileSelected(Vec<File>),
    FileLoaded(Vec<u8>),
    ProcessPdf(u32),
    PdfProcessed(Result<Vec<u8>, String>),
    SetDpi(u32),
    UpdateProgress(String),
}

pub struct App {
    file: Option<Vec<u8>>,
    processing: bool,
    result: Option<Result<Vec<u8>, String>>,
    file_reader: Option<FileReader>,
    dpi: u32,
    file_name: Option<String>,
    progress_message: Option<String>,
}

impl Component for App {
    type Message = Msg;
    type Properties = ();

    fn create(_ctx: &Context<Self>) -> Self {
        Self {
            file: None,
            processing: false,
            result: None,
            file_reader: None,
            dpi: 72,
            file_name: None,
            progress_message: None,
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        match msg {
            Msg::FileSelected(files) => {
                if let Some(file) = files.first() {
                    self.file_name = Some(file.name());
                    let link = ctx.link().clone();
                    let file_reader = gloo_file::callbacks::read_as_bytes(file, move |res| {
                        if let Ok(data) = res {
                            link.send_message(Msg::FileLoaded(data));
                        }
                    });
                    self.file_reader = Some(file_reader);
                }
                true
            }
            Msg::FileLoaded(data) => {
                log!("ファイルを読み込みました");
                self.file = Some(data);
                self.result = None;
                self.progress_message = None;
                true
            }
            Msg::ProcessPdf(dpi) => {
                if let Some(data) = &self.file {
                    self.processing = true;
                    self.result = None;
                    self.progress_message = Some("処理を開始しています...".to_string());
                    log!(format!("PDFを処理中... (DPI: {})", dpi));

                    let data = data.clone();
                    let link = ctx.link().clone();

                    // WASMで処理を実行
                    wasm_bindgen_futures::spawn_local(async move {
                        let result = crate::rasterize_pdf_with_progress(data, dpi, {
                            let link = link.clone();
                            move |msg| {
                                link.send_message(Msg::UpdateProgress(msg));
                            }
                        })
                        .await
                        .map_err(|e| format!("エラー: {}", e));
                        link.send_message(Msg::PdfProcessed(result));
                    });
                }
                true
            }
            Msg::PdfProcessed(result) => {
                self.processing = false;
                self.progress_message = None;
                match &result {
                    Ok(_) => log!("PDF処理が完了しました"),
                    Err(e) => log!(format!("エラー: {}", e)),
                }
                self.result = Some(result);
                true
            }
            Msg::SetDpi(dpi) => {
                self.dpi = dpi;
                true
            }
            Msg::UpdateProgress(message) => {
                self.progress_message = Some(message);
                true
            }
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let on_file_change = {
            let link = ctx.link().clone();
            Callback::from(move |e: Event| {
                let input: HtmlInputElement = e.target().unwrap().dyn_into().unwrap();
                if let Some(files) = input.files() {
                    let file_list: Vec<File> = js_sys::try_iter(&files)
                        .unwrap()
                        .unwrap()
                        .map(|v| File::from(web_sys::File::from(v.unwrap())))
                        .collect();
                    link.send_message(Msg::FileSelected(file_list));
                }
            })
        };

        let on_process = {
            let link = ctx.link().clone();
            let dpi = self.dpi;
            Callback::from(move |_| {
                link.send_message(Msg::ProcessPdf(dpi));
            })
        };

        let on_dpi_change = {
            let link = ctx.link().clone();
            Callback::from(move |e: Event| {
                let input: HtmlInputElement = e.target().unwrap().dyn_into().unwrap();
                if let Ok(value) = input.value().parse::<u32>() {
                    link.send_message(Msg::SetDpi(value));
                }
            })
        };

        let download_button = if let Some(Ok(data)) = &self.result {
            let data = data.clone();
            let file_name = self
                .file_name
                .as_ref()
                .and_then(|name| name.strip_suffix(".pdf"))
                .map(|base| format!("{}_rasterized.pdf", base))
                .unwrap_or_else(|| "output.pdf".to_string());

            html! {
                <button
                    class="download-button"
                    onclick={Callback::from(move |_| {
                        download_pdf(&data, &file_name);
                    })}
                >
                    { "ダウンロード" }
                </button>
            }
        } else {
            html! {}
        };

        html! {
            <div class="container">
                <header class="header">
                    <h1>{ "PDF Rasterizer" }</h1>
                    <p class="subtitle">{ "PDFを画像化してから再度PDFに変換するツール" }</p>
                </header>

                <main class="main">
                    <div class="upload-section">
                        <label class="file-label">
                            <input
                                type="file"
                                accept=".pdf"
                                onchange={on_file_change}
                                class="file-input"
                            />
                            <span class="file-button">{ "PDFを選択" }</span>
                        </label>
                        {
                            if let Some(name) = &self.file_name {
                                html! { <p class="file-name">{ format!("選択: {}", name) }</p> }
                            } else {
                                html! {}
                            }
                        }
                    </div>

                    <div class="settings-section">
                        <label class="dpi-label">
                            { "DPI: " }
                            <input
                                type="number"
                                value={self.dpi.to_string()}
                                onchange={on_dpi_change}
                                min="72"
                                max="600"
                                step="1"
                                class="dpi-input"
                            />
                        </label>
                        <p class="dpi-hint">{ "解像度を指定します（72-600）" }</p>
                    </div>

                    <div class="action-section">
                        <button
                            class="process-button"
                            onclick={on_process}
                            disabled={self.file.is_none() || self.processing}
                        >
                            {
                                if self.processing {
                                    "処理中..."
                                } else {
                                    "変換"
                                }
                            }
                        </button>
                    </div>

                    {
                        if let Some(progress) = &self.progress_message {
                            html! {
                                <div class="progress">
                                    <div class="progress-spinner"></div>
                                    <p>{ progress }</p>
                                </div>
                            }
                        } else {
                            html! {}
                        }
                    }

                    {
                        if let Some(Err(e)) = &self.result {
                            html! {
                                <div class="error">
                                    <p>{ e }</p>
                                </div>
                            }
                        } else {
                            html! {}
                        }
                    }

                    {
                        if self.result.as_ref().map(|r| r.is_ok()).unwrap_or(false) {
                            html! {
                                <div class="success">
                                    <p>{ "✓ 変換完了" }</p>
                                    { download_button }
                                </div>
                            }
                        } else {
                            html! {}
                        }
                    }
                </main>

                <footer class="footer">
                    <p>
                        { "技術スタック: " }
                        <a href="https://github.com/SergiusIW/hayro" target="_blank">{ "hayro" }</a>
                        { " | " }
                        <a href="https://github.com/fschutt/printpdf" target="_blank">{ "printpdf" }</a>
                        { " | " }
                        <a href="https://yew.rs" target="_blank">{ "Yew" }</a>
                    </p>
                </footer>
            </div>
        }
    }
}

fn download_pdf(data: &[u8], filename: &str) {
    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();

    // Blobを作成
    let array = js_sys::Uint8Array::new(&unsafe { js_sys::Uint8Array::view(data) }.into());
    let blob_parts = js_sys::Array::new();
    blob_parts.push(&array.buffer());

    let blob_property = web_sys::BlobPropertyBag::new();
    blob_property.set_type("application/pdf");

    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&blob_parts, &blob_property)
        .unwrap();

    // URLを作成
    let url = web_sys::Url::create_object_url_with_blob(&blob).unwrap();

    // ダウンロードリンクを作成してクリック
    let a = document
        .create_element("a")
        .unwrap()
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .unwrap();
    a.set_href(&url);
    a.set_download(filename);
    a.click();

    // URLを解放
    web_sys::Url::revoke_object_url(&url).unwrap();
}
