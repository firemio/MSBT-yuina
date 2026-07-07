# MSBT-yuina 画像ビューアー
![MSBT-yuina](./assets/yuina.webp)

rust製の軽量画像ビューアーです。画像をウィンドウサイズに合わせて表示し、ズームやドラッグで自由に閲覧できます。
透過バックグラウンドに対応しています。
いにしえのビューワーMassiGraに影響を受けています。

## 主な機能

- 画像をウィンドウサイズに自動でフィット
- マウスホイールでズーム（0.02倍〜64倍／2%〜6400%）
- SVG はベクターのまま、どの倍率でも線とテキストが鮮明
  （可視領域だけを表示解像度ちょうどで都度ラスタライズ）
- SVG のテキスト描画に対応（システムフォント＋ `@font-face` 埋め込みフォント）
- 90°単位の回転（L/R キー）
- ドラッグで画像を移動、ダブルクリックでフィット⇔100%切り替え
- 左右キー等で同じフォルダ内の画像を切り替え（エクスプローラー風の自然順）
- F11 で全画面表示
- コマンドライン引数で画像を直接開く
- 自動更新（起動時に新バージョンを確認し、メニューからワンクリックで更新）

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
- SVG (.svg, .svgz) — ベクターのまま、拡大しても鮮明。テキスト（システムフォント／@font-face 埋め込みフォント）対応
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
- **+ / -**: ズームイン／ズームアウト
- **← →** / **PageUp PageDown** / **Space Backspace**: 同じフォルダ内の画像を切り替え
- **Home / End**: フォルダ内の最初／最後の画像へ
- **L / R**: 左／右に90°回転
- **F11**: 全画面表示の切り替え
- **O**: ファイルを開く
- **Esc**: アプリケーションを終了


### コマンドラインから開く

```bash
# Windows
MSBT-yuina.exe path/to/image.jpg

# Linux/macOS
./MSBT-yuina path/to/image.jpg
```

### マウス操作

- **ズーム**: マウスホイール（カーソル位置を基準に拡大縮小）
- **移動**: 画像をドラッグ
- **フィット⇔100%切り替え**: ダブルクリック
- **次の画像 / 前の画像**: 右矢印キー / 左矢印キー

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

[Releases](../../releases) ページから入手できます（インストール不要のポータブルアプリ）。

- `MSBT-yuina-vX.Y.Z-win64.zip` — exe＋設定ファイルのセット（推奨）
- `MSBT-yuina.exe` — 実行ファイル単体
- `SHA256SUMS.txt` — ダウンロード検証用チェックサム

好きな場所に置いて実行するだけです。画像ファイルを exe にドラッグするか、関連付けて開けます。

> **初回起動時に「WindowsによってPCが保護されました」（SmartScreen）と表示された場合**
> 本アプリはコード署名証明書を持たない個人開発のフリーソフトのため、この警告が出ます。
> 「詳細情報」→「実行」で起動できます（初回のみ）。心配な場合は `SHA256SUMS.txt` で
> ダウンロードしたファイルのハッシュを確認してください:
> `Get-FileHash MSBT-yuina.exe`（PowerShell）
>
> なお MSIX 形式は署名証明書がないと Windows がインストール自体を拒否するため採用していません。

### 自動更新

起動時に GitHub Releases の新バージョンを自動確認します（`check_updates = false` で無効化可能）。
新バージョンがあるとメニューバー右端に「⬆ Update vX.Y.Z」ボタンが出ます。クリックすると
ダウンロード → exe の差し替えまで自動で行われ、再起動すると新バージョンになります。
手動で確認する場合は **Help > Check for Updates**。

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
