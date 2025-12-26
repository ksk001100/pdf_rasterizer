# pdf_rasterizer

PDFファイルを画像化してから再度PDFに変換するツール

## 特徴

- **CLIとWebアプリの両対応**: コマンドラインツールとブラウザアプリとして利用可能
- **純粋なRust実装**: hayroを使用してPDFをレンダリング
- **外部依存なし**: すべての処理をRustライブラリで実行
- **高品質**: DPI指定で解像度を調整可能
- **ブラウザで完結**: WebAssemblyでブラウザ上で動作（サーバーにアップロード不要）

## インストール

### 推奨方法: cargo install

```bash
# リポジトリをクローン
git clone https://github.com/ksk001100/pdf_rasterizer.git
cd pdf_rasterizer

# インストール
cargo install --path .
```

### 開発者向け: ローカルビルド

```bash
# ビルド
cargo build --release

# 実行
./target/release/pdf_rasterizer input.pdf output.pdf
```

## 使い方

### Webアプリケーション（推奨）

GitHub Pagesでホストされているウェブアプリを利用できます：

**https://ksk001100.github.io/pdf_rasterizer/**

1. PDFファイルを選択
2. 必要に応じてDPI（解像度）を調整
3. 「変換」ボタンをクリック
4. 変換されたPDFをダウンロード

すべての処理はブラウザ内で完結し、ファイルがサーバーにアップロードされることはありません。

### CLIツール

```bash
pdf_rasterizer [OPTIONS] <入力PDF> <出力PDF>
```

#### オプション

- `--dpi <DPI>`: ラスタライズ時の解像度（デフォルト: 72）

#### 例

```bash
# デフォルト設定（DPI: 72、元のサイズを維持）
pdf_rasterizer input.pdf output.pdf

# 高解像度（DPI: 300）
pdf_rasterizer --dpi 300 input.pdf output.pdf
```

## ユースケース

- **互換性の向上**: 複雑なPDFをシンプルな画像ベースPDFに変換
- **ファイルサイズの削減**: 過剰に複雑なPDFを軽量化
- **レンダリング問題の解決**: 一部のビューアで表示できないPDFを修正

**注意**: テキスト選択などのインタラクティブ機能は失われます

## 開発

### Webアプリケーションのローカル実行

```bash
# Trunkをインストール
cargo install trunk

# WASMターゲットを追加
rustup target add wasm32-unknown-unknown

# 開発サーバーを起動
trunk serve
```

ブラウザで http://localhost:8080 を開いてアプリケーションを確認できます。

### プロダクションビルド

```bash
trunk build --release
```

ビルドされたファイルは `dist/` ディレクトリに出力されます。

## 技術スタック

### フロントエンド (WebAssembly)
- **Yew**: Rustで書かれたモダンなWebフレームワーク
- **Trunk**: WASMアプリケーションのビルドツール
- **gloo**: Web APIのRustラッパー

### バックエンド / PDF処理
- **hayro**: 純粋なRust実装のPDFレンダリングライブラリ
- **lopdf**: PDF生成・操作ライブラリ
- **image**: 画像処理

## ライセンス

このプロジェクトはMITライセンスの下で公開されています。
