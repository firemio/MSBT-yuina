# MSBT-yuina 画像ビューアー

rust製の軽量画像ビューアーです。画像をウィンドウサイズに合わせて表示し、ズームやドラッグで自由に閲覧できます。
透過バックグラウンドに対応しています。

## 主な機能

- 画像をウィンドウサイズに自動でフィット
- マウスホイールでズーム（0.01倍から10倍）
- ドラッグで画像を移動
- 左右キーで同じフォルダ内の画像を切り替え
- コマンドライン引数で画像を直接開く

## 対応フォーマット

- JPEG (.jpg, .jpeg)
- PNG (.png)
- GIF (.gif)
- WebP (.webp)
- BMP (.bmp)

## 使い方

### GUIから開く

1. アプリケーションを起動
2. 「Open Image」ボタンをクリックまたは画像ファイルをドラッグ&ドロップ
3. 画像を選択

### コマンドラインから開く

```bash
# Windows
simple-image-viewer.exe path/to/image.jpg

# Linux/macOS
./simple-image-viewer path/to/image.jpg
```

### 操作方法

- **ズーム**: マウスホイール
- **移動**: 画像をドラッグ
- **次の画像**: 右矢印キー
- **前の画像**: 左矢印キー

## インストール

### バイナリをダウンロード

[Releases](../../releases)ページから、お使いのプラットフォーム用のバイナリをダウンロードしてください。

### ソースからビルド

```bash
# 開発版をビルド
cargo build

# リリース版をビルド（推奨）
cargo build --release
```

## 技術情報

- 言語: Rust
- GUI: egui
- 画像処理: image-rs

## ライセンス

MIT License
