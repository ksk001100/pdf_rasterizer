# pdf_rasterizer

PDFファイルを画像化してから再度PDFに変換するCLIツール

## 特徴

- **シンプルなインストール**: `cargo install` だけで完結
- **純粋なRust実装**: hayroを使用してPDFをレンダリング
- **外部依存なし**: すべての処理をRustライブラリで実行
- **高品質**: DPI指定で解像度を調整可能

## インストール

### 推奨方法: cargo install

```bash
# リポジトリをクローン
git clone https://github.com/your-username/pdf_rasterizer.git
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

```bash
pdf_rasterizer [OPTIONS] <入力PDF> <出力PDF>
```

### オプション

- `--dpi <DPI>`: ラスタライズ時の解像度（デフォルト: 72）

### 例

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

## 技術スタック

- **hayro**: 純粋なRust実装のPDFレンダリングライブラリ
- **printpdf**: PDF生成
- **image**: 画像処理

## ライセンス

このプロジェクトはMITライセンスの下で公開されています。
