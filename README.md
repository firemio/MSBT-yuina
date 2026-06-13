# MSBT-yuina 画像ビューアー
![MSBT-yuina](./assets/yuina.webp)

rust製の軽量画像ビューアーです。画像をウィンドウサイズに合わせて表示し、ズームやドラッグで自由に閲覧できます。
透過バックグラウンドに対応しています。
いにしえのビューワーMassiGraに影響を受けています。

## 主な機能

- 画像をウィンドウサイズに自動でフィット
- マウスホイールでズーム（0.1倍〜10倍／10%〜1000%）
- SVG はベクターのまま、拡大しても鮮明（表示解像度で都度ラスタライズ）
- ドラッグで画像を移動
- 左右キーで同じフォルダ内の画像を切り替え
- コマンドライン引数で画像を直接開く

## 対応フォーマット

主要な画像フォーマットを幅広くサポートしています。

- JPEG (.jpg, .jpeg, .jfif)
- PNG (.png)
- GIF (.gif)
- WebP (.webp)
- BMP (.bmp, .dib)
- TIFF (.tif, .tiff)
- ICO (.ico)
- TGA (.tga) / DDS (.dds) / QOI (.qoi)
- PNM 系 (.pnm, .ppm, .pgm, .pbm) / farbfeld (.ff)
- OpenEXR (.exr) / Radiance HDR (.hdr)
- SVG (.svg) — ベクターのまま、拡大しても鮮明
- HEIC/HEIF (.heic, .heif) — iPhone等の写真（後述の前提条件あり）
- AVIF (.avif) / JPEG XR (.jxr, .wdp) — Windows のコーデックを利用（後述）

上記以外でも、Windows の画像コーデック（WIC）が対応していれば開ける場合があります
（File ダイアログの「All Files」で選択、またはウィンドウにドラッグ＆ドロップ）。

## 使い方

### GUIから開く

1. アプリケーションを起動
2. メニューの **File > Open** から開く、または画像ファイルをウィンドウにドラッグ&ドロップ

### キーボード操作

- **F**: 画像をウィンドウサイズに自動でフィット
- **0**: 画像を100%にズーム
- **←→**: 左右キーで同じフォルダ内の画像を切り替え
- **O**: ファイルを開く
- **Esc**: アプリケーションを終了


### コマンドラインから開く

```bash
# Windows
MSBT-yuina.exe path/to/image.jpg

# Linux/macOS
./MSBT-yuina path/to/image.jpg
```

### 操作方法

- **ズーム**: マウスホイール
- **移動**: 画像をドラッグ
- **次の画像**: 右矢印キー
- **前の画像**: 左矢印キー

### マウスジェスチャー

- **右ドラッグで左方向**: 前の画像へ移動
- **右ドラッグで右方向**: 次の画像へ移動

※ 右クリックを押しながら方向に動かし、離すとアクションが実行されます

### HEIC / AVIF など OS コーデックを使う形式について

HEIC/HEIF や AVIF・JPEG XR などは、Windows 標準の画像コーデック（WIC）を使ってデコードします。
追加のライブラリ同梱は不要で exe 単体のままですが、形式によっては Microsoft Store の拡張機能
（いずれも無料）が必要です。多くの Windows 10/11 では導入済みです。

- HEIC/HEIF: [HEIF 画像拡張機能](https://apps.microsoft.com/detail/9pmmsr1cgpwg)
- AVIF: [AV1 ビデオ拡張機能](https://apps.microsoft.com/detail/9mvzqvxjbq9v)

未導入の形式を開くとエラー表示になります（アプリは落ちません）。

## インストール

### バイナリをダウンロード（Windows）

[Releases](../../releases) ページから最新の `MSBT-yuina.exe` をダウンロードして実行するだけです（インストール不要）。画像ファイルを exe にドラッグするか、関連付けて開けます。

### ソースからビルド

```bash
# 開発版をビルド
cargo build

# リリース版をビルド（推奨）
cargo build --release
```

## 技術情報

- 言語: Rust
- GUI: egui / eframe
- 画像処理: image-rs
- SVGサポート: resvg / usvg / tiny-skia（表示解像度でラスタライズし、拡大しても鮮明）
- HEICサポート: Windows WIC（OS の HEIF コーデックを利用、追加ライブラリ不要）

## ライセンス

MIT License
