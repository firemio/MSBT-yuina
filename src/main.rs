#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, Key, Pos2, Rect, Vec2};
use egui::IconData;
use ico;
use image;
use log::{debug, error, info, LevelFilter};
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
};
use resvg;
use rfd;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs;
use std::io::Cursor;
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use resvg::tiny_skia::{self, Pixmap};
use usvg::{Options, Tree};
// vello はバージョン整合のため vello_svg の再エクスポートを使う
use vello_svg::vello;

mod updater;
use updater::UpdateStatus;

/// 対応する画像拡張子。image クレートでデコードできるもの（png/jpg/gif/webp/bmp/tiff/ico/tga/
/// dds/exr/hdr/qoi/pnm 系）に加え、WIC フォールバックで開ける形式（heic/heif/avif/jxr 等）と
/// svg/svgz。File ダイアログのフィルタとフォルダ送りの判定で共通利用する。
const SUPPORTED_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "jfif", "gif", "webp", "bmp", "dib", "svg", "svgz", "heic", "heif",
    "avif", "tif", "tiff", "ico", "tga", "dds", "exr", "hdr", "qoi", "pnm", "ppm", "pgm", "pbm",
    "pam", "ff", "farbfeld", "jxr", "wdp",
];

/// 拡大率の下限・上限
const MIN_SCALE: f32 = 0.02;
const MAX_SCALE: f32 = 64.0;
/// +/- キー1回あたりのズーム倍率
const KEY_ZOOM_STEP: f32 = 1.2;
/// SVG のパン中の再レンダリング回数を減らすため、可視領域の外側に付ける描画余白（物理px）
const SVG_RENDER_MARGIN_PX: f32 = 256.0;

/// 設定ファイル（TOML）の内容
#[derive(Serialize, Deserialize, Debug)]
pub struct ViewerConfig {
    /// "fit" でウィンドウサイズに合わせて表示、"original" で画像本来のサイズで表示
    pub initial_display_mode: String,
    /// デバッグログを有効にするかどうか
    #[serde(default)]
    pub enable_debug_log: bool,
    /// マウスホイールの拡大率（1回の回転あたりの倍率）
    #[serde(default = "default_wheel_zoom_factor")]
    pub wheel_zoom_factor: f32,
    /// 拡大表示の補間。true=滑らか（バイリニア）、false=ピクセル等倍（ニアレスト）
    #[serde(default = "default_true")]
    pub smooth_zoom: bool,
    /// 起動時に GitHub Releases の新バージョンを確認するかどうか
    #[serde(default = "default_true")]
    pub check_updates: bool,
    /// SVG を GPU（vello/wgpu）でラスタライズするかどうか。
    /// 失敗時や未対応機能を含む SVG では自動的に CPU（resvg）へフォールバックする
    #[serde(default = "default_true")]
    pub gpu_rendering: bool,
}

fn default_wheel_zoom_factor() -> f32 {
    0.001
}

fn default_true() -> bool {
    true
}

impl Default for ViewerConfig {
    fn default() -> Self {
        Self {
            initial_display_mode: "fit".to_string(),
            enable_debug_log: false,
            wheel_zoom_factor: default_wheel_zoom_factor(),
            smooth_zoom: true,
            check_updates: true,
            gpu_rendering: true,
        }
    }
}

impl ViewerConfig {
    fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let exe_path = std::env::current_exe()?;
        let exe_name = exe_path
            .file_stem()
            .ok_or("Failed to get executable name")?
            .to_string_lossy();
        let config_file = format!("{}.toml", exe_name);
        let config_str = fs::read_to_string(&config_file)?;
        let config: ViewerConfig = toml::from_str(&config_str)?;
        info!("設定を読み込みました: wheel_zoom_factor = {}", config.wheel_zoom_factor);
        Ok(config)
    }

    fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let exe_path = std::env::current_exe()?;
        let exe_name = exe_path
            .file_stem()
            .ok_or("Failed to get executable name")?
            .to_string_lossy();
        let config_file = format!("{}.toml", exe_name);

        // 設定ファイルのテンプレート
        let config_template = format!(
            "# 初期表示モード: \"fit\"=画面に合わせて表示, \"original\"=原寸大(100%)\n\
             initial_display_mode = \"{}\"\n\
             \n\
             # デバッグログを有効にするかどうか\n\
             enable_debug_log = {}\n\
             \n\
             # マウスホイールの拡大率（1回の回転あたりの倍率）\n\
             wheel_zoom_factor = {}\n\
             \n\
             # 拡大表示の補間: true=滑らか(バイリニア), false=ピクセル等倍(ニアレスト)\n\
             smooth_zoom = {}\n\
             \n\
             # 起動時に新バージョンを確認するかどうか\n\
             check_updates = {}\n\
             \n\
             # SVGをGPUで描画するかどうか（失敗時は自動でCPUに切り替え）\n\
             gpu_rendering = {}\n",
            self.initial_display_mode,
            self.enable_debug_log,
            self.wheel_zoom_factor,
            self.smooth_zoom,
            self.check_updates,
            self.gpu_rendering
        );

        fs::write(config_file, config_template)?;
        Ok(())
    }
}

fn init_logging(config: &ViewerConfig) -> Result<(), Box<dyn std::error::Error>> {
    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path.parent().ok_or("Failed to get executable directory")?;
    let exe_name = exe_path
        .file_stem()
        .ok_or("Failed to get executable name")?
        .to_string_lossy();
    let log_path = exe_dir.join(format!("{}.log", exe_name));

    let file_appender = FileAppender::builder()
        .encoder(Box::new(PatternEncoder::new("{d(%Y-%m-%d %H:%M:%S)} - {l} - {m}\n")))
        .build(log_path)?;

    let level = if config.enable_debug_log {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };

    let config = Config::builder()
        .appender(Appender::builder().build("file", Box::new(file_appender)))
        .build(Root::builder()
            .appender("file")
            .build(level))?;
    log4rs::init_config(config)?;
    Ok(())
}

fn main() -> eframe::Result<()> {
    // パニック時のログ出力
    panic::set_hook(Box::new(|panic_info| {
        error!("アプリケーションがパニックで終了: {}", panic_info);
    }));

    // 設定ファイルの読み込み
    let config = match ViewerConfig::load() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("設定ファイルの読み込みに失敗しました: {}。デフォルト設定を使用します。", e);
            ViewerConfig::default()
        }
    };

    // ログの初期化
    if let Err(e) = init_logging(&config) {
        eprintln!("ログの初期化に失敗: {}", e);
        rfd::MessageDialog::new()
            .set_title("エラー")
            .set_description(&format!("ログの初期化に失敗: {}", e))
            .show();
        return Ok(());
    }
    info!("アプリケーション起動開始");

    // コマンドライン引数を取得
    let args: Vec<String> = std::env::args().collect();
    let initial_image = if args.len() > 1 {
        let path = PathBuf::from(&args[1]);
        if !path.exists() {
            let message = format!("指定された画像が見つかりません: {}", path.display());
            error!("{}", message);
            eprintln!("エラー: {}", message);
            rfd::MessageDialog::new()
                .set_title("エラー")
                .set_description(&message)
                .show();
        }
        Some(path)
    } else {
        None
    };

    if let Some(path) = &initial_image {
        info!("コマンドライン引数で指定された画像: {}", path.display());
    }

    let options = match create_app_options() {
        Ok(opt) => opt,
        Err(e) => {
            let message = format!("アプリケーション設定の作成に失敗: {}", e);
            error!("{}", message);
            eprintln!("エラー: {}", message);
            rfd::MessageDialog::new()
                .set_title("エラー")
                .set_description(&message)
                .show();
            return Ok(());
        }
    };
    info!("アプリケーション設定の作成完了");

    eframe::run_native(
        "MSBT-yuina",
        options,
        Box::new(move |cc| {
            info!("アプリケーションコンテキストの作成開始");
            Ok(Box::new(ImageViewer::new(cc, initial_image, config)))
        }),
    )
}

fn create_app_options() -> Result<eframe::NativeOptions, Box<dyn std::error::Error>> {
    info!("アプリケーション設定の作成開始");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([200.0, 200.0])
            .with_drag_and_drop(true)
            .with_title("MSBT-yuina")
            .with_icon(load_icon())
            .with_transparent(false)
            .with_decorations(true)
            .with_visible(true),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    Ok(options)
}

/// SVG テクスチャが現在保持している描画領域。
/// crop は「回転適用後の表示空間」における物理 px の矩形 [x, y, w, h]（画像原点基準）。
#[derive(Clone, Copy, PartialEq)]
struct SvgView {
    /// SVG のユーザー単位 → 物理 px の倍率（表示倍率 × pixels_per_point）
    scale_px: f32,
    /// 90°単位の回転（0..=3、時計回り）
    rot: u8,
    crop: [u32; 4],
}

/// バックグラウンドワーカーへの SVG ラスタライズ要求
struct SvgRenderJob {
    scale_px: f32,
    rot: u8,
    /// 表示空間（回転後）の crop [x,y,w,h]（物理px）
    crop: [u32; 4],
    /// SVG 空間の crop（レンダリングに使う矩形）
    svg_crop: [f32; 4],
}

/// ワーカーからのラスタライズ結果
struct SvgRenderResult {
    view: SvgView,
    image: egui::ColorImage,
}

/// 木のすべてのグループが「合成に影響する属性を持たない」か確認する。
/// true なら、子ノードを個別に（間のグループを無視して）描画しても結果が変わらないため、
/// 視野外ノードのカリングが安全にできる。opacity/フィルタ/マスク/クリップ/ブレンドの
/// いずれかを持つグループがあれば false（従来どおり木全体を描画する）。
fn tree_is_flat(group: &usvg::Group) -> bool {
    group.children().iter().all(|node| match node {
        usvg::Node::Group(g) => {
            g.opacity().get() == 1.0
                && g.blend_mode() == usvg::BlendMode::Normal
                && g.clip_path().is_none()
                && g.mask().is_none()
                && g.filters().is_empty()
                && !g.isolate()
                && tree_is_flat(g)
        }
        _ => true,
    })
}

/// 可視範囲（SVG絶対ユーザー座標）と交差するノードだけを描画する（ビューポートカリング）。
/// resvg::render は毎回すべての要素を処理し、しかもストロークの輪郭生成コストは
/// ズーム倍率に比例して増えるため、拡大するほど遅くなる。ここで視野外の要素を
/// バウンディングボックス判定で描画前に間引くことで、拡大時ほど軽くする。
/// tree_is_flat() が true の木でのみ正しい結果になる。
fn render_culled(
    group: &usvg::Group,
    clip: tiny_skia::Rect,
    ts: usvg::Transform,
    pixmap: &mut tiny_skia::PixmapMut,
    drawn: &mut u32,
    culled: &mut u32,
) {
    for node in group.children() {
        let bbox = node.abs_stroke_bounding_box();
        let visible = bbox.left() < clip.right()
            && bbox.right() > clip.left()
            && bbox.top() < clip.bottom()
            && bbox.bottom() > clip.top();
        if !visible {
            *culled += 1;
            continue;
        }
        match node {
            usvg::Node::Group(g) => render_culled(g, clip, ts, pixmap, drawn, culled),
            _ => {
                // resvg::render_node は「ノードを自身の bbox 原点へ平行移動して描く」
                // 単体描画用の API で、祖先グループの変換も適用しない。
                // その平行移動を打ち消し（pre_translate で相殺）、祖先の変換
                // （abs_transform）を合成することで、resvg::render による
                // ツリー全体描画とまったく同じ位置・見た目で描く。
                if let Some(layer_bbox) = node.abs_layer_bounding_box() {
                    let p = ts
                        .pre_concat(node.abs_transform())
                        .pre_translate(layer_bbox.x(), layer_bbox.y());
                    resvg::render_node(node, p, pixmap);
                    *drawn += 1;
                }
            }
        }
    }
}

/// vello（GPU）が解釈できない機能（フィルタ・マスク）を木が使っているか。
/// 使っている場合は正確性のため CPU（resvg）で描画する。
fn tree_uses_filters_or_masks(group: &usvg::Group) -> bool {
    group.children().iter().any(|node| match node {
        usvg::Node::Group(g) => {
            !g.filters().is_empty() || g.mask().is_some() || tree_uses_filters_or_masks(g)
        }
        _ => false,
    })
}

/// 可視範囲と交差するノードだけを vello::Scene へ組み立てる（GPU 版ビューポートカリング）。
/// vello は画面外のジオメトリもデバイス空間で処理するため、高倍率で全要素を
/// シーンに入れると内部バッファが溢れて何も描画されなくなる。フラットな木では
/// ここで可視要素だけを組み立てることで、拡大時ほどシーンが小さく・速く・安全になる。
/// 個々のノードの描画は vello_svg::render_group（Apache-2.0/MIT）の忠実な移植。
fn vello_append_culled(
    scene: &mut vello::Scene,
    group: &usvg::Group,
    view: vello::kurbo::Affine,
    clip: tiny_skia::Rect,
    drawn: &mut u32,
    culled: &mut u32,
) {
    for node in group.children() {
        let bbox = node.abs_stroke_bounding_box();
        let visible = bbox.left() < clip.right()
            && bbox.right() > clip.left()
            && bbox.top() < clip.bottom()
            && bbox.bottom() > clip.top();
        if !visible {
            *culled += 1;
            continue;
        }
        match node {
            // フラットな木なのでグループはレイヤー（合成）を持たない。中へ降りるだけ
            usvg::Node::Group(g) => vello_append_culled(scene, g, view, clip, drawn, culled),
            _ => {
                vello_append_node(scene, node, view);
                *drawn += 1;
            }
        }
    }
}

/// 単一ノード（Path/Image/Text）を vello::Scene へ追加する。
/// vello_svg 0.9 の render.rs と同じセマンティクス（塗り規則・描画順・ブラシ変換）。
fn vello_append_node(scene: &mut vello::Scene, node: &usvg::Node, view: vello::kurbo::Affine) {
    use vello::peniko::Fill;
    use vello_svg::util;
    let transform = view * util::to_affine(&node.abs_transform());
    match node {
        usvg::Node::Group(g) => {
            // テキスト展開内などに現れる合成付きグループ。レイヤーを張って中を描く
            let alpha = g.opacity().get();
            let bb = g.layer_bounding_box();
            let rect = vello::kurbo::Rect::from_origin_size(
                (bb.x() as f64, bb.y() as f64),
                (bb.width() as f64, bb.height() as f64),
            );
            scene.push_layer(Fill::NonZero, vello::peniko::Mix::Normal, alpha, transform, &rect);
            for child in g.children() {
                vello_append_node(scene, child, view);
            }
            scene.pop_layer();
        }
        usvg::Node::Path(path) => {
            if !path.is_visible() {
                return;
            }
            let local_path = util::to_bez_path(path);
            let do_fill = |scene: &mut vello::Scene| {
                if let Some(fill) = &path.fill() {
                    if let Some((brush, brush_transform)) =
                        util::to_brush(fill.paint(), fill.opacity())
                    {
                        scene.fill(
                            match fill.rule() {
                                usvg::FillRule::NonZero => Fill::NonZero,
                                usvg::FillRule::EvenOdd => Fill::EvenOdd,
                            },
                            transform,
                            &brush,
                            Some(brush_transform),
                            &local_path,
                        );
                    }
                }
            };
            let do_stroke = |scene: &mut vello::Scene| {
                if let Some(stroke) = &path.stroke() {
                    if let Some((brush, brush_transform)) =
                        util::to_brush(stroke.paint(), stroke.opacity())
                    {
                        let conv_stroke = util::to_stroke(stroke);
                        scene.stroke(&conv_stroke, transform, &brush, Some(brush_transform), &local_path);
                    }
                }
            };
            match path.paint_order() {
                usvg::PaintOrder::FillAndStroke => {
                    do_fill(scene);
                    do_stroke(scene);
                }
                usvg::PaintOrder::StrokeAndFill => {
                    do_stroke(scene);
                    do_fill(scene);
                }
            }
        }
        usvg::Node::Image(img) => {
            if !img.is_visible() {
                return;
            }
            match img.kind() {
                usvg::ImageKind::JPEG(_)
                | usvg::ImageKind::PNG(_)
                | usvg::ImageKind::GIF(_)
                | usvg::ImageKind::WEBP(_) => {
                    if let Ok(decoded) = util::decode_raw_raster_image(img.kind()) {
                        let image = util::into_image(decoded);
                        scene.draw_image(&image, transform);
                    }
                }
                usvg::ImageKind::SVG(svg) => {
                    for child in svg.root().children() {
                        vello_append_node(scene, child, transform);
                    }
                }
            }
        }
        usvg::Node::Text(text) => {
            for child in text.flattened().children() {
                vello_append_node(scene, child, transform);
            }
        }
    }
}

/// GPU（vello/wgpu）による SVG ラスタライザ。
/// ヘッドレスの wgpu デバイスを持ち、vello::Scene を GPU コンピュートで
/// ラスタライズして結果を CPU へ読み戻す。読み戻しは画面サイズ程度（数MB）なので
/// 数msで済み、ラスタライズ本体が大規模並列になるぶん圧倒的に速い。
struct GpuRenderer {
    device: vello::wgpu::Device,
    queue: vello::wgpu::Queue,
    renderer: vello::Renderer,
    /// SVG 全体を変換済みのシーン（非フラットな木でのみ遅延構築）
    whole_scene: Option<vello::Scene>,
    adapter_name: String,
}

impl GpuRenderer {
    fn new() -> Result<Self, String> {
        use vello::wgpu;
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("GPUアダプタが見つかりません: {e}"))?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("MSBT-yuina svg renderer"),
            ..Default::default()
        }))
        .map_err(|e| format!("GPUデバイスの作成に失敗: {e}"))?;
        let renderer = vello::Renderer::new(&device, vello::RendererOptions::default())
            .map_err(|e| format!("vello の初期化に失敗: {e}"))?;
        Ok(Self {
            device,
            queue,
            renderer,
            whole_scene: None,
            adapter_name: info.name,
        })
    }

    /// 1 ジョブぶんをレンダリングする。戻り値は (画像, 描画ノード数, カリング数)。
    /// flat な木では可視ノードだけのシーンを組み立てる（GPU 版カリング）。
    fn render(
        &mut self,
        tree: &Tree,
        flat: bool,
        job: &SvgRenderJob,
    ) -> Result<(egui::ColorImage, u32, u32), String> {
        use vello::wgpu;
        let pw = job.svg_crop[2].round().max(1.0) as u32;
        let ph = job.svg_crop[3].round().max(1.0) as u32;

        // SVG座標 → crop 内 px（回転は描画時の UV で表現するため含めない）
        let s = job.scale_px as f64;
        let affine = vello::kurbo::Affine::new([
            s,
            0.0,
            0.0,
            s,
            -(job.svg_crop[0] as f64),
            -(job.svg_crop[1] as f64),
        ]);
        let mut scene = vello::Scene::new();
        let (mut drawn, mut culled) = (0u32, 0u32);
        if flat {
            // 可視範囲（ユーザー座標）。AA のにじみ分を少し広げる
            let pad = 2.0 / job.scale_px.max(f32::EPSILON);
            let clip = tiny_skia::Rect::from_xywh(
                job.svg_crop[0] / job.scale_px - pad,
                job.svg_crop[1] / job.scale_px - pad,
                job.svg_crop[2] / job.scale_px + pad * 2.0,
                job.svg_crop[3] / job.scale_px + pad * 2.0,
            )
            .ok_or("可視範囲の計算に失敗")?;
            vello_append_culled(&mut scene, tree.root(), affine, clip, &mut drawn, &mut culled);
        } else {
            let whole = self
                .whole_scene
                .get_or_insert_with(|| vello_svg::render_tree(tree));
            scene.append(whole, Some(affine));
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("svg_target"),
            size: wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                &scene,
                &view,
                &vello::RenderParams {
                    base_color: vello::peniko::Color::TRANSPARENT,
                    width: pw,
                    height: ph,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .map_err(|e| format!("GPUレンダリングに失敗: {e}"))?;

        // GPU → CPU 読み戻し（bytes_per_row は 256 バイト境界に揃える必要がある）
        let bpr = (pw * 4).next_multiple_of(256);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("svg_readback"),
            size: bpr as u64 * ph as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (map_tx, map_rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = map_tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|e| format!("GPUの完了待ちに失敗: {e:?}"))?;
        map_rx
            .recv()
            .map_err(|_| "GPU読み戻しの完了通知が来ません".to_string())?
            .map_err(|e| format!("GPU読み戻しに失敗: {e:?}"))?;

        let data = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((pw * ph * 4) as usize);
        for row in 0..ph {
            let start = (row * bpr) as usize;
            pixels.extend_from_slice(&data[start..start + (pw * 4) as usize]);
        }
        drop(data);
        buffer.unmap();
        // vello の出力は premultiplied RGBA
        Ok((
            egui::ColorImage::from_rgba_premultiplied([pw as usize, ph as usize], &pixels),
            drawn,
            culled,
        ))
    }
}

/// SVG ラスタライズ用のワーカースレッドを起動する。
/// 精密な SVG は 1 回のラスタライズに時間がかかるため UI スレッドでは行わず、
/// ここで処理する。チャンネルに溜まった要求は最新の 1 件だけを処理する
/// （連続ズーム中の中間状態は描いても無駄になるので捨てる）。
/// 送信側（LoadedImage::Svg）が破棄されるとスレッドは自動終了する。
fn spawn_svg_render_worker(
    tree: Tree,
    use_gpu: bool,
    ctx: egui::Context,
) -> (mpsc::Sender<SvgRenderJob>, mpsc::Receiver<SvgRenderResult>) {
    let (job_tx, job_rx) = mpsc::channel::<SvgRenderJob>();
    let (result_tx, result_rx) = mpsc::channel::<SvgRenderResult>();
    std::thread::spawn(move || {
        // フラットな木（グループ効果なし）なら CPU パスで視野外カリングが安全に使える
        let flat = tree_is_flat(tree.root());
        // GPU: vello が解釈できる木なら GPU ラスタライザを初期化。
        // 失敗・未対応時は CPU（resvg）へフォールバックする。
        let mut gpu = if use_gpu && !tree_uses_filters_or_masks(tree.root()) {
            match GpuRenderer::new() {
                Ok(g) => {
                    info!("SVGレンダラ: GPU — vello / {}", g.adapter_name);
                    Some(g)
                }
                Err(e) => {
                    info!("GPU初期化に失敗したため CPU で描画します: {e}");
                    None
                }
            }
        } else {
            if use_gpu {
                info!("フィルタ/マスクを含む SVG のため CPU（resvg）で描画します");
            }
            None
        };
        if gpu.is_none() {
            info!(
                "SVGレンダラ: CPU — resvg{}",
                if flat { "（視野外カリング有効）" } else { "" }
            );
        }
        while let Ok(mut job) = job_rx.recv() {
            // 最新の要求だけを残す
            while let Ok(newer) = job_rx.try_recv() {
                job = newer;
            }
            let pw = job.svg_crop[2].round().max(1.0) as u32;
            let ph = job.svg_crop[3].round().max(1.0) as u32;
            let started = std::time::Instant::now();

            // まず GPU で試し、失敗したら以後は CPU に切り替える。
            // 非フラットな木は全体シーンを append する方式のため、デバイス空間の全体サイズが
            // 大きすぎると vello の内部バッファが溢れて空の出力になる。その場合は CPU を使う。
            let mut image: Option<egui::ColorImage> = None;
            let mut backend = "GPU";
            let (mut drawn, mut culled) = (0u32, 0u32);
            let gpu_usable_now = flat || {
                let size = tree.size();
                size.width() * job.scale_px <= 4096.0 && size.height() * job.scale_px <= 4096.0
            };
            if let Some(g) = gpu.as_mut() {
                if gpu_usable_now {
                    match g.render(&tree, flat, &job) {
                        Ok((img, d, c)) => {
                            image = Some(img);
                            drawn = d;
                            culled = c;
                        }
                        Err(e) => {
                            error!("GPU描画に失敗したため、以後 CPU で描画します: {e}");
                            gpu = None;
                        }
                    }
                }
            }
            let image = match image {
                Some(img) => img,
                None => {
                    backend = "CPU";
                    let Some(mut pixmap) = Pixmap::new(pw, ph) else {
                        error!("SVGレンダリング用のバッファを確保できません: {}x{}", pw, ph);
                        continue;
                    };
                    // SVG座標 → crop 内 px。回転は描画時の UV で表現するため含めない
                    let ts = usvg::Transform::from_scale(job.scale_px, job.scale_px)
                        .post_translate(-job.svg_crop[0], -job.svg_crop[1]);
                    // 可視範囲をユーザー座標へ戻し、AA のにじみ分だけ少し広げる
                    let pad = 2.0 / job.scale_px.max(f32::EPSILON);
                    let clip = tiny_skia::Rect::from_xywh(
                        job.svg_crop[0] / job.scale_px - pad,
                        job.svg_crop[1] / job.scale_px - pad,
                        job.svg_crop[2] / job.scale_px + pad * 2.0,
                        job.svg_crop[3] / job.scale_px + pad * 2.0,
                    );
                    match (flat, clip) {
                        (true, Some(clip)) => render_culled(
                            tree.root(),
                            clip,
                            ts,
                            &mut pixmap.as_mut(),
                            &mut drawn,
                            &mut culled,
                        ),
                        _ => resvg::render(&tree, ts, &mut pixmap.as_mut()),
                    }
                    // tiny-skia の出力は premultiplied RGBA なのでそのまま渡す
                    egui::ColorImage::from_rgba_premultiplied(
                        [pw as usize, ph as usize],
                        pixmap.data(),
                    )
                }
            };
            debug!(
                "SVGレンダリング[{}]: {}x{} scale_px={:.2} ({} ms, 描画 {} / カリング {})",
                backend,
                pw,
                ph,
                job.scale_px,
                started.elapsed().as_millis(),
                drawn,
                culled
            );
            let result = SvgRenderResult {
                view: SvgView {
                    scale_px: job.scale_px,
                    rot: job.rot,
                    crop: job.crop,
                },
                image,
            };
            if result_tx.send(result).is_err() {
                break; // 受信側が破棄済み（画像が切り替わった）
            }
            ctx.request_repaint();
        }
    });
    (job_tx, result_rx)
}

/// 読み込んだ画像の種類を表す型
/// Raster: 通常画像
/// Svg: 専用ワーカースレッドが可視領域だけを表示解像度でラスタライズする。
///      UI スレッドは要求を送り、完成までは手持ちのテクスチャを引き伸ばして表示する
enum LoadedImage {
    Raster {
        texture: egui::TextureHandle,
        path: PathBuf,
    },
    Svg {
        /// SVG 本来のサイズ（ユーザー単位 ＝ 等倍時の論理 px）
        size: [f32; 2],
        /// 直近にラスタライズされた可視領域のテクスチャ（初回結果の到着時に生成）
        texture: Option<egui::TextureHandle>,
        /// texture が保持している領域の情報
        view: Option<SvgView>,
        /// ラスタライズ要求の送信先（ワーカースレッド）
        job_tx: mpsc::Sender<SvgRenderJob>,
        /// ラスタライズ結果の受信元
        result_rx: mpsc::Receiver<SvgRenderResult>,
        /// 直近にワーカーへ依頼した内容（同一要求の重複送信を防ぐ）
        last_requested: Option<SvgView>,
        path: PathBuf,
    },
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum GestureDirection {
    Left,
    Right,
    Up,
    Down,
}

struct MouseGesture {
    is_active: bool,
    last_pos: Option<Vec2>,
    directions: Vec<GestureDirection>,
    threshold: f32,
}

impl MouseGesture {
    fn new() -> Self {
        Self {
            is_active: false,
            last_pos: None,
            directions: Vec::new(),
            threshold: 20.0, // スワイプを検出する最小距離
        }
    }

    fn reset(&mut self) {
        self.is_active = false;
        self.last_pos = None;
        self.directions.clear();
    }

    fn get_action(&self) -> Option<String> {
        if self.directions.len() == 1 {
            match self.directions[0] {
                GestureDirection::Left => Some("<<<".to_string()),
                GestureDirection::Right => Some(">>>".to_string()),
                _ => None,
            }
        } else {
            None
        }
    }

    fn update(&mut self, mouse_pos: Vec2, right_button: bool) -> Option<String> {
        let mut action = None;
        if right_button {
            if !self.is_active {
                self.is_active = true;
                self.last_pos = Some(mouse_pos);
            } else if let Some(last_pos) = self.last_pos {
                let delta = mouse_pos - last_pos;
                if delta.length() >= self.threshold {
                    let direction = if delta.x.abs() > delta.y.abs() {
                        if delta.x > 0.0 {
                            Some(GestureDirection::Right)
                        } else {
                            Some(GestureDirection::Left)
                        }
                    } else {
                        if delta.y > 0.0 {
                            Some(GestureDirection::Down)
                        } else {
                            Some(GestureDirection::Up)
                        }
                    };

                    if let Some(dir) = direction {
                        if self.directions.len() < 5 &&
                           self.directions.last().map_or(true, |last| *last != dir) {
                            self.directions.push(dir);
                        }
                    }
                    self.last_pos = Some(mouse_pos);
                }
            }
        } else if self.is_active {
            // 右クリックが離されたときにアクションを取得
            action = self.get_action();
            self.reset();
        }
        action
    }

    fn draw(&self, ui: &mut egui::Ui, center: Pos2) {
        if !self.is_active {
            return;
        }

        let painter = ui.painter();
        let text_color = Color32::from_rgb(255, 255, 255);
        let arrow_color = Color32::from_rgb(200, 200, 200);
        let font_size = 24.0;
        let spacing = 30.0;

        // 半透明の黒背景を描画
        let background_rect = Rect::from_center_size(
            center,
            Vec2::new(spacing * 5.0, font_size * 3.0),
        );
        painter.rect_filled(
            background_rect,
            0.0,
            Color32::from_rgba_premultiplied(0, 0, 0, 180),
        );

        // 方向の矢印を描画
        for (i, direction) in self.directions.iter().enumerate().take(5) {
            let pos = center + Vec2::new((i as f32 - 2.0) * spacing, 0.0);
            let arrow = match direction {
                GestureDirection::Left => "←",
                GestureDirection::Right => "→",
                GestureDirection::Up => "↑",
                GestureDirection::Down => "↓",
            };
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                arrow,
                egui::FontId::monospace(font_size),
                arrow_color,
            );
        }

        // アクションを描画
        if let Some(action) = self.get_action() {
            painter.text(
                center + Vec2::new(0.0, 30.0),
                egui::Align2::CENTER_CENTER,
                &action,
                egui::FontId::monospace(font_size),
                text_color,
            );
        }
    }
}

/// 90°単位の回転を適用した (幅, 高さ)
fn rotated_dims(w: f32, h: f32, rot: u8) -> (f32, f32) {
    if rot % 2 == 1 {
        (h, w)
    } else {
        (w, h)
    }
}

/// 表示空間（時計回りに rot×90° 回転した後の空間）の矩形 [x,y,w,h] を、
/// 回転前の SVG 空間の矩形へ写像する。ws/hs は SVG 空間の全体サイズ。
/// 単位は呼び出し側で一貫していれば何でもよい（ここでは物理 px を渡す）。
fn map_display_crop_to_svg(rot: u8, ws: f32, hs: f32, crop: [f32; 4]) -> [f32; 4] {
    let [dx, dy, dw, dh] = crop;
    match rot % 4 {
        0 => [dx, dy, dw, dh],
        // 90° 時計回り: SVG の (x,y) は表示 (hs - y, x) に現れる
        1 => [dy, hs - dx - dw, dh, dw],
        2 => [ws - dx - dw, hs - dy - dh, dw, dh],
        // 270° 時計回り: SVG の (x,y) は表示 (y, ws - x) に現れる
        _ => [ws - dy - dh, dx, dh, dw],
    }
}

/// テクスチャを rot×90°（時計回り）回転させて rect に描画する。
/// egui::Image は非正方形の 90° 回転を素直に扱えないため、UV を回した Mesh で描く。
fn draw_texture_rotated(painter: &egui::Painter, texture: &egui::TextureHandle, rect: Rect, rot: u8) {
    use egui::epaint::{Mesh, Vertex};
    let mut mesh = Mesh::with_texture(texture.id());
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    let base = [
        egui::pos2(0.0, 0.0),
        egui::pos2(1.0, 0.0),
        egui::pos2(1.0, 1.0),
        egui::pos2(0.0, 1.0),
    ];
    for (i, corner) in corners.iter().enumerate() {
        mesh.vertices.push(Vertex {
            pos: *corner,
            uv: base[(i + 4 - rot as usize % 4) % 4],
            color: Color32::WHITE,
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// SVG 描画範囲の1軸ぶんを決める。可視区間 [n0, n1] を優先しつつ余白 margin を付け、
/// 画像全体 full と GPU 上限 max_dim に収める。戻り値は (開始, 幅) の整数px。
/// 可視区間そのものが max_dim を超える場合は中央の max_dim 窓に絞る（単一テクスチャでは
/// 描ききれないため。この場合でも戻り値は決定的なので、ビューが動かない限り再描画されない）。
fn crop_axis(n0: f32, n1: f32, full: f32, margin: f32, max_dim: f32) -> (u32, u32) {
    let n0 = n0.clamp(0.0, full);
    let n1 = n1.clamp(n0, full);
    let (v0, v1) = if n1 - n0 > max_dim {
        let start = ((n0 + n1 - max_dim) * 0.5)
            .clamp(0.0, (full - max_dim).max(0.0))
            .floor();
        (start, start + max_dim)
    } else {
        (n0, n1)
    };
    // 余白は max_dim に収まる範囲でだけ付ける（左右均等）
    let slack = (max_dim - (v1 - v0)).max(0.0);
    let m = margin.min(slack * 0.5);
    let c0 = (v0 - m).max(0.0).floor();
    let c1 = (v1 + m).min(full).ceil();
    let w = ((c1 - c0).round() as u32).clamp(1, max_dim as u32);
    (c0 as u32, w)
}

/// エクスプローラー風の自然順ソート比較（数値の並びを数として比較、英字は大文字小文字無視）。
/// 例: img2.png < img10.png
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    let mut na = String::new();
                    while let Some(&c) = ai.peek() {
                        if c.is_ascii_digit() {
                            na.push(c);
                            ai.next();
                        } else {
                            break;
                        }
                    }
                    let mut nb = String::new();
                    while let Some(&c) = bi.peek() {
                        if c.is_ascii_digit() {
                            nb.push(c);
                            bi.next();
                        } else {
                            break;
                        }
                    }
                    // 先頭ゼロを除いた桁数 → 辞書順 → 元の長さ（"01" と "1" の安定化）で比較
                    let ta = na.trim_start_matches('0');
                    let tb = nb.trim_start_matches('0');
                    let ord = ta
                        .len()
                        .cmp(&tb.len())
                        .then_with(|| ta.cmp(tb))
                        .then_with(|| na.len().cmp(&nb.len()));
                    if ord != Ordering::Equal {
                        return ord;
                    }
                } else {
                    let la = ca.to_lowercase().next().unwrap_or(ca);
                    let lb = cb.to_lowercase().next().unwrap_or(cb);
                    if la != lb {
                        return la.cmp(&lb);
                    }
                    ai.next();
                    bi.next();
                }
            }
        }
    }
}

/// gzip 圧縮されたデータ（.svgz）なら展開して返す。それ以外はそのまま返す。
fn decompress_if_gzip(raw: Vec<u8>) -> Result<Vec<u8>, String> {
    if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        use std::io::Read as _;
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(raw.as_slice())
            .read_to_end(&mut out)
            .map_err(|e| format!("svgz の展開に失敗しました: {e}"))?;
        Ok(out)
    } else {
        Ok(raw)
    }
}

/// fontdb が読めるフォント形式（TTF/OTF/TTC）かをマジックバイトで判定する。
/// WOFF/WOFF2 は変換が必要なため対象外。
fn is_supported_font(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && matches!(&bytes[..4], b"\x00\x01\x00\x00" | b"OTTO" | b"true" | b"ttcf")
}

/// SVG 中の @font-face 宣言から埋め込みフォント（data URI）やローカルファイル参照を抽出し、
/// フォント DB へ登録する。usvg は @font-face を解釈しないため、ここで補う。
/// 読み込めたフォント数を返す。
fn load_embedded_fonts(svg_text: &str, db: &mut usvg::fontdb::Database, base_dir: Option<&Path>) -> usize {
    use base64::Engine as _;
    let mut loaded = 0;
    let mut rest = svg_text;
    while let Some(pos) = rest.find("@font-face") {
        rest = &rest[pos + "@font-face".len()..];
        let Some(open) = rest.find('{') else { break };
        let Some(close_rel) = rest[open..].find('}') else { break };
        let block = &rest[open + 1..open + close_rel];

        // ブロック内の url(...) を列挙する
        let mut b = block;
        while let Some(u) = b.find("url(") {
            b = &b[u + 4..];
            let Some(end) = b.find(')') else { break };
            let raw_url = b[..end]
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .trim();
            if let Some(data) = raw_url.strip_prefix("data:") {
                // data URI（base64 埋め込みフォント）
                if let Some(comma) = data.find(',') {
                    let (meta, payload) = data.split_at(comma);
                    if meta.contains("base64") {
                        let cleaned: String =
                            payload[1..].chars().filter(|c| !c.is_whitespace()).collect();
                        match base64::engine::general_purpose::STANDARD.decode(cleaned.as_bytes()) {
                            Ok(bytes) if is_supported_font(&bytes) => {
                                db.load_font_data(bytes);
                                loaded += 1;
                            }
                            Ok(_) => {
                                info!("@font-face: 未対応のフォント形式（WOFF/WOFF2 等）をスキップしました");
                            }
                            Err(e) => info!("@font-face: base64 のデコードに失敗: {e}"),
                        }
                    }
                }
            } else if !raw_url.contains("://") {
                // SVG ファイルからの相対パス参照
                if let Some(dir) = base_dir {
                    let font_path = dir.join(raw_url);
                    if let Ok(bytes) = fs::read(&font_path) {
                        if is_supported_font(&bytes) {
                            db.load_font_data(bytes);
                            loaded += 1;
                        }
                    }
                }
            }
            b = &b[end..];
        }
        rest = &rest[open + close_rel..];
    }
    loaded
}

struct ImageViewer {
    config: ViewerConfig,
    current_image: Option<LoadedImage>,
    current_path: Option<PathBuf>,
    image_size: Option<[u32; 2]>,
    scale: f32,
    pan_offset: Vec2,
    /// 90°単位の回転（0..=3、時計回り）。画像を読み込むたびに 0 へ戻る
    rotation: u8,
    image_paths: Vec<PathBuf>,
    // 前回の利用可能なウィンドウサイズ（"fit" モードで使用）
    last_available_size: Option<Vec2>,
    mouse_gesture: MouseGesture,
    // SVG テキスト描画用のフォントDB（初回SVG読み込み時にシステムフォントを一度だけロードしてキャッシュ）
    fontdb: Option<Arc<usvg::fontdb::Database>>,
    // コマンドライン等で指定された初期画像。最初の update() フレームで読み込む（new() 内で
    // 読み込むと、GL バックエンドが最大テクスチャサイズを報告する前なので、大きな画像で
    // 「maximum texture side is 2048」パニックが起きる）。
    pending_open: Option<PathBuf>,
    // 直近に設定したウィンドウタイトル（毎フレームの Title コマンド送信を避ける）
    last_title: String,
    // 自動更新の状態（バックグラウンドスレッドと共有）
    update_status: updater::SharedStatus,
    // 起動時の自分自身のパス。exe 差し替え後の再起動に使う
    // （self-replace 後の current_exe() はリネーム後のパスを返し得るため起動時に確保する）
    exe_path: Option<PathBuf>,
    // 更新適用後の再起動確認ダイアログを一度だけ出すためのフラグ
    restart_prompted: bool,
}

impl ImageViewer {
    fn new(cc: &eframe::CreationContext<'_>, initial_image: Option<PathBuf>, config: ViewerConfig) -> Self {
        let mut viewer = Self {
            config,
            current_image: None,
            current_path: None,
            image_size: None,
            scale: 1.0,
            pan_offset: Vec2::ZERO,
            rotation: 0,
            image_paths: Vec::new(),
            last_available_size: None,
            mouse_gesture: MouseGesture::new(),
            fontdb: None,
            pending_open: None,
            last_title: String::new(),
            update_status: updater::new_shared_status(),
            exe_path: std::env::current_exe().ok(),
            restart_prompted: false,
        };

        // 初期画像は new() 内ではなく最初の update() フレームで読み込む（理由は pending_open の定義参照）。
        if let Some(path) = initial_image {
            if path.exists() {
                viewer.pending_open = Some(path);
            } else {
                error!("指定された画像が見つかりません: {}", path.display());
            }
        }

        // 起動時の更新確認（バックグラウンド。失敗しても Help メニューに出るだけでアプリは動く）
        if viewer.config.check_updates {
            updater::spawn_check(viewer.update_status.clone(), cc.egui_ctx.clone());
        }

        viewer
    }

    /// 更新の確認ダイアログを出し、承諾されたらダウンロードと差し替えを開始する
    fn start_update(&mut self, version: String, url: String, ctx: &egui::Context) {
        let Some(exe) = self.exe_path.clone() else {
            rfd::MessageDialog::new()
                .set_title("アップデート")
                .set_description("実行ファイルのパスを取得できないため更新できません")
                .show();
            return;
        };
        let answer = rfd::MessageDialog::new()
            .set_title("アップデート")
            .set_description(&format!(
                "新しいバージョン {version} をダウンロードして適用しますか？\n（適用後、再起動すると新しいバージョンになります）"
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        if answer == rfd::MessageDialogResult::Yes {
            updater::spawn_download_and_install(
                self.update_status.clone(),
                version,
                url,
                exe,
                ctx.clone(),
            );
        }
    }

    /// 新しい exe で自分自身を起動し直し、このプロセスを終了する
    fn restart_to_apply_update(&mut self, ctx: &egui::Context) {
        let Some(exe) = self.exe_path.clone() else { return };
        let mut cmd = std::process::Command::new(&exe);
        // いま表示している画像を引き継ぐ
        if let Some(p) = &self.current_path {
            cmd.arg(p);
        }
        match cmd.spawn() {
            Ok(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(e) => {
                let message = format!("再起動に失敗しました: {e}\n手動でアプリを起動し直してください。");
                error!("{}", message);
                rfd::MessageDialog::new()
                    .set_title("アップデート")
                    .set_description(&message)
                    .show();
            }
        }
    }

    /// 回転を考慮した、拡大率 1.0 のときの表示サイズ（論理ポイント）
    fn display_base_size(&self) -> Vec2 {
        let (w, h) = match &self.current_image {
            Some(LoadedImage::Raster { texture, .. }) => {
                let s = texture.size_vec2();
                (s.x, s.y)
            }
            Some(LoadedImage::Svg { size, .. }) => (size[0], size[1]),
            None => (0.0, 0.0),
        };
        let (w, h) = rotated_dims(w, h, self.rotation);
        Vec2::new(w, h)
    }

    /// 画像（回転考慮）が利用可能領域全体に収まる scale を計算する
    fn fit_to_screen(&mut self, avail: Vec2) {
        let base = self.display_base_size();
        if base.x > 0.0 && base.y > 0.0 && avail.x > 0.0 && avail.y > 0.0 {
            self.scale = (avail.x / base.x).min(avail.y / base.y);
            info!("画面に合わせてスケールを設定: {}", self.scale);
        }
    }

    /// anchor（スクリーン座標）の位置にある画像上の点を固定したままズームする
    fn zoom_at(&mut self, anchor: Vec2, panel_rect: &Rect, base_size: Vec2, new_scale: f32) {
        let old_scale = self.scale;
        if old_scale <= 0.0 || (new_scale - old_scale).abs() < f32::EPSILON {
            return;
        }
        let old_size = base_size * old_scale;
        let new_size = base_size * new_scale;
        let old_origin = panel_rect.min.to_vec2() + (panel_rect.size() - old_size) * 0.5 + self.pan_offset;
        let new_origin = anchor - (anchor - old_origin) * (new_scale / old_scale);
        self.scale = new_scale;
        self.pan_offset =
            new_origin - panel_rect.min.to_vec2() - (panel_rect.size() - new_size) * 0.5;
    }

    /// 指定パスの画像を読み込み、拡大率、パン位置、画像サイズを更新する
    fn load_image(&mut self, path: &Path, ctx: &egui::Context) -> bool {
        info!("画像を読み込もうとしています: {:?}", path);
        self.pan_offset = Vec2::ZERO;
        self.scale = 1.0;
        self.rotation = 0;
        self.image_size = None;

        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let result = if ext == "svg" || ext == "svgz" {
            self.load_svg(path, ctx)
        } else {
            // 拡張子が不明でも WIC フォールバック込みでラスタとして試す
            self.load_raster(path, ctx)
        };

        if result {
            self.current_path = Some(path.to_path_buf());
            // フィット表示は次フレームの update() に委ねる。
            // ここで（＝ImageViewer::new() からの初回読み込み時に）fit_to_screen を呼ぶと
            // Context::run() 前に ctx.available_rect() を呼ぶことになり、egui 0.31 が
            // 「Called `available_rect()` before `Context::run()`」でパニックする。
            // last_available_size を None に戻すと、次フレームで新しい画像に対して再フィットされる。
            self.last_available_size = None;
        }

        result
    }

    fn load_svg(&mut self, path: &Path, ctx: &egui::Context) -> bool {
        match self.try_load_svg(path, ctx) {
            Ok(()) => true,
            Err(message) => {
                error!("{}", message);
                rfd::MessageDialog::new()
                    .set_title("エラー")
                    .set_description(&message)
                    .show();
                false
            }
        }
    }

    fn try_load_svg(&mut self, path: &Path, ctx: &egui::Context) -> Result<(), String> {
        let raw = fs::read(path)
            .map_err(|e| format!("SVGファイルの読み込みに失敗しました: {} - {}", path.display(), e))?;
        // .svgz（gzip 圧縮 SVG）対応
        let raw = decompress_if_gzip(raw)?;
        let svg_text = String::from_utf8_lossy(&raw);
        info!("SVGファイルを読み込みました: {} bytes", svg_text.len());

        // SVG 内のテキストはパース時にフォントDBを使ってパス化される。
        // フォントDBが空だと文字が一切描画されないため、システムフォントをロードしておく。
        // 構築コストが高いので初回のみ作成し、以降は Arc を共有して再利用する。
        let fontdb = self
            .fontdb
            .get_or_insert_with(|| {
                let mut db = usvg::fontdb::Database::new();
                db.load_system_fonts();
                info!("システムフォントをロードしました: {} faces", db.len());
                Arc::new(db)
            })
            .clone();

        // @font-face による埋め込み・参照フォントがあれば、システムフォントDBの
        // コピーへ追加登録して使う（usvg 自身は @font-face を解釈しない）。
        let fontdb = if svg_text.contains("@font-face") {
            let mut db = (*fontdb).clone();
            let n = load_embedded_fonts(&svg_text, &mut db, path.parent());
            info!("@font-face から {} 個のフォントを読み込みました", n);
            Arc::new(db)
        } else {
            fontdb
        };

        let mut opt = Options::default();
        opt.fontdb = fontdb;
        // SVG から相対参照される画像などの解決基準ディレクトリ
        opt.resources_dir = path.parent().map(|p| p.to_path_buf());

        let tree = Tree::from_str(&svg_text, &opt)
            .map_err(|e| format!("SVGの解析に失敗しました: {} - {}", path.display(), e))?;
        let size = tree.size();
        let (w, h) = (size.width(), size.height());
        info!("SVGサイズ: {}x{}", w, h);
        self.image_size = Some([w.ceil() as u32, h.ceil() as u32]);
        // Tree はワーカースレッドへ移動し、以後のラスタライズはすべてそちらで行う。
        // 前の画像のワーカーは、旧 LoadedImage が破棄されて送信側が閉じると自動終了する。
        let (job_tx, result_rx) =
            spawn_svg_render_worker(tree, self.config.gpu_rendering, ctx.clone());
        self.current_image = Some(LoadedImage::Svg {
            size: [w, h],
            texture: None,
            view: None,
            job_tx,
            result_rx,
            last_requested: None,
            path: path.to_path_buf(),
        });
        info!("SVGの読み込みが完了しました");
        Ok(())
    }

    fn load_raster(&mut self, path: &Path, ctx: &egui::Context) -> bool {
        // まず image クレートで読む（png/jpg/gif/webp/bmp/tiff/ico/tga/dds/exr/hdr/qoi/pnm 等を網羅）。
        // image が非対応の形式（HEIC/HEIF/AVIF/JPEG XR/カメラRAW 等）は、Windows の WIC
        // （OS が持つ画像コーデック＋ストアの拡張機能）にフォールバックして可能な限り開く。
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let decoded: Result<image::DynamicImage, String> = if ext == "heic" || ext == "heif" {
            decode_via_wic(path) // image は HEIC 非対応なので最初から WIC
        } else {
            match image::open(path) {
                Ok(img) => Ok(img),
                Err(e) => {
                    info!("image で読めず WIC にフォールバック: {} ({})", path.display(), e);
                    decode_via_wic(path).map_err(|werr| format!("{e} / WIC: {werr}"))
                }
            }
        };
        match decoded {
            Ok(mut image) => {
                // GPU の最大テクスチャ辺を超える画像はそのまま load_texture するとパニックするため、
                // アスペクト比を保ったまま収まるよう縮小する（通常サイズの画像には影響しない）。
                let max = ctx.input(|i| i.max_texture_side).max(1) as u32;
                if image.width() > max || image.height() > max {
                    info!(
                        "画像がテクスチャ上限({0})を超過: {1}x{2} → 縮小",
                        max,
                        image.width(),
                        image.height()
                    );
                    image = image.resize(max, max, image::imageops::FilterType::Triangle);
                }
                let image = image.to_rgba8();
                let width = image.width() as usize;
                let height = image.height() as usize;
                self.image_size = Some([width as u32, height as u32]);
                // フィットは load_image 側で last_available_size=None を介して次フレームに委ねる
                // （ここで fit_to_screen を呼ぶと初回フレーム前に available_rect() を触り得る）
                let size = [width, height];
                let pixels = image.into_vec();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                // 拡大時の補間は設定で切り替え（滑らか／ピクセル等倍）。縮小は常にバイリニア。
                let magnification = if self.config.smooth_zoom {
                    egui::TextureFilter::Linear
                } else {
                    egui::TextureFilter::Nearest
                };
                let texture = ctx.load_texture(
                    path.to_string_lossy().to_string(),
                    color_image,
                    egui::TextureOptions {
                        magnification,
                        minification: egui::TextureFilter::Linear,
                        ..Default::default()
                    },
                );
                self.current_image = Some(LoadedImage::Raster {
                    texture,
                    path: path.to_path_buf(),
                });
                info!("画像の読み込みが完了しました");
                true
            }
            Err(e) => {
                let message = format!("画像の読み込みに失敗しました: {} - {}", path.display(), e);
                error!("{}", message);
                rfd::MessageDialog::new()
                    .set_title("エラー")
                    .set_description(&message)
                    .show();
                false
            }
        }
    }

    /// 現在の画像があるディレクトリ内の画像一覧を更新する
    fn update_image_list(&mut self, current_path: &Path) {
        if let Some(parent) = current_path.parent() {
            info!("ディレクトリを読み込もうとしています: {:?}", parent);
            match fs::read_dir(parent) {
                Ok(entries) => {
                    let mut files: Vec<_> = entries
                        .filter_map(|entry| {
                            match entry {
                                Ok(entry) => {
                                    let path = entry.path();
                                    if path.extension().map_or(false, |ext| {
                                        let ext = ext.to_string_lossy().to_lowercase();
                                        SUPPORTED_EXTS.contains(&ext.as_str())
                                    }) {
                                        Some(path)
                                    } else {
                                        None
                                    }
                                }
                                Err(e) => {
                                    error!("ディレクトリエントリの読み込みに失敗: {}", e);
                                    None
                                }
                            }
                        })
                        .collect();
                    // エクスプローラーと同じ感覚で並ぶよう自然順（img2 < img10）でソート
                    files.sort_by(|a, b| {
                        let an = a.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        let bn = b.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        natural_cmp(&an, &bn).then_with(|| a.cmp(b))
                    });
                    self.image_paths = files;
                    info!("ディレクトリの読み込みに成功しました: {:?}", parent);
                }
                Err(e) => {
                    error!("ディレクトリの読み込みに失敗しました: {:?} - エラー: {}", parent, e);
                }
            }
        }
    }

    /// 前後の画像へ切り替え
    fn load_adjacent_image(&mut self, ctx: &egui::Context, next: bool) {
        if let Some(current_path) = &self.current_path {
            let current_index = self
                .image_paths
                .iter()
                .position(|p| p == current_path)
                .unwrap_or(0);
            let new_index = if next {
                if current_index + 1 < self.image_paths.len() {
                    current_index + 1
                } else {
                    0
                }
            } else {
                if current_index > 0 {
                    current_index - 1
                } else {
                    self.image_paths.len().saturating_sub(1)
                }
            };
            if let Some(path) = self.image_paths.get(new_index).cloned() {
                self.load_image(&path, ctx);
            }
        }
    }

    /// フォルダ内の指定インデックスの画像へ切り替え（Home/End 用）
    fn load_image_at(&mut self, ctx: &egui::Context, index: usize) {
        if let Some(path) = self.image_paths.get(index).cloned() {
            if Some(&path) != self.current_path.as_ref() {
                self.load_image(&path, ctx);
            }
        }
    }

    /// ファイルダイアログで画像を開く
    fn open_file_dialog(&mut self, ctx: &egui::Context) {
        if let Some(file_path) = rfd::FileDialog::new()
            .add_filter("Images", SUPPORTED_EXTS)
            .add_filter("All Files", &["*"])
            .pick_file()
        {
            self.load_image(&file_path, ctx);
            self.update_image_list(&file_path);
        }
    }

    /// アプリケーション更新処理
    /// ・ドラッグ＆ドロップによるファイル読み込み
    /// ・メニューバー（File / Options）の表示
    /// ・"fit" モードの場合、ウィンドウサイズ変更時に scale 再計算
    /// ・SVG は毎フレーム可視領域だけを表示解像度でラスタライズし、どの倍率でも線が鮮明なまま
    /// ・キー操作: ←→/PgUp/PgDn/Space/BS=前後, Home/End=先頭末尾, F=フィット, 0=100%,
    ///   +/-=ズーム, L/R=回転, F11=全画面, O=開く, Esc=終了
    fn update(&mut self, ctx: &egui::Context) {
        // 初期画像の遅延読み込み（最初のフレームで一度だけ）。
        // ここなら GL バックエンドが報告した正しい max_texture_side が使えるため、
        // 大きな画像でもパニックしない。take() で一度きりにしてエラー時の無限リトライも防ぐ。
        if let Some(path) = self.pending_open.take() {
            self.load_image(&path, ctx);
            self.update_image_list(&path);
        }

        // ドラッグ＆ドロップ対応（複数ドロップ時は先頭のみ開く。同フォルダの残りは前後送りで辿れる）
        let dropped = ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone()));
        if let Some(path) = dropped {
            self.load_image(&path, ctx);
            self.update_image_list(&path);
        }

        // メニューバー（File / Options）の表示
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open... (O)").clicked() {
                        self.open_file_dialog(ctx);
                        ui.close_menu();
                    }
                });
                ui.menu_button("Options", |ui| {
                    ui.group(|ui| {
                        ui.label("Display Mode");
                        ui.separator();
                        ui.radio_value(
                            &mut self.config.initial_display_mode,
                            "fit".to_string(),
                            "Fit Window",
                        );
                        ui.radio_value(
                            &mut self.config.initial_display_mode,
                            "original".to_string(),
                            "Original Size",
                        );
                    });

                    ui.add_space(8.0);

                    ui.group(|ui| {
                        ui.label("Zoom");
                        ui.separator();
                        if ui
                            .checkbox(&mut self.config.smooth_zoom, "Smooth magnification")
                            .changed()
                        {
                            // ラスタ画像のテクスチャフィルタは生成時に決まるため、読み直して反映する
                            if let Some(LoadedImage::Raster { path, .. }) = &self.current_image {
                                let path = path.clone();
                                let keep = (
                                    self.scale,
                                    self.pan_offset,
                                    self.last_available_size,
                                    self.rotation,
                                );
                                if self.load_image(&path, ctx) {
                                    (
                                        self.scale,
                                        self.pan_offset,
                                        self.last_available_size,
                                        self.rotation,
                                    ) = keep;
                                }
                            }
                        }
                        ui.add(
                            egui::Slider::new(&mut self.config.wheel_zoom_factor, 0.0002..=0.005)
                                .logarithmic(true)
                                .text("Wheel zoom speed"),
                        );
                        if ui
                            .checkbox(&mut self.config.gpu_rendering, "GPU SVG rendering")
                            .changed()
                        {
                            // ワーカーの構成が変わるため SVG を読み直して反映する
                            if let Some(LoadedImage::Svg { path, .. }) = &self.current_image {
                                let path = path.clone();
                                let keep = (
                                    self.scale,
                                    self.pan_offset,
                                    self.last_available_size,
                                    self.rotation,
                                );
                                if self.load_image(&path, ctx) {
                                    (
                                        self.scale,
                                        self.pan_offset,
                                        self.last_available_size,
                                        self.rotation,
                                    ) = keep;
                                }
                            }
                        }
                    });

                    ui.add_space(8.0);

                    ui.group(|ui| {
                        ui.label("Other Settings");
                        ui.separator();
                        ui.checkbox(&mut self.config.enable_debug_log, "Enable Debug Log");
                        ui.checkbox(&mut self.config.check_updates, "Check updates on startup");
                    });

                    ui.add_space(8.0);

                    if ui.button("Save Settings").clicked() {
                        match self.config.save() {
                            Ok(_) => info!("設定が保存されました"),
                            Err(e) => error!("設定の保存に失敗しました: {}", e),
                        }
                        ui.close_menu();
                    }
                });
                ui.menu_button("Help", |ui| {
                    ui.label(format!("MSBT-yuina v{}", updater::CURRENT_VERSION));
                    ui.separator();
                    let status = self.update_status.lock().unwrap().clone();
                    match status {
                        UpdateStatus::Checking => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("更新を確認中…");
                            });
                        }
                        UpdateStatus::Downloading { version } => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(format!("{version} をダウンロード中…"));
                            });
                        }
                        UpdateStatus::Available { version, url } => {
                            if ui.button(format!("Update to {version}")).clicked() {
                                ui.close_menu();
                                self.start_update(version, url, ctx);
                            }
                        }
                        UpdateStatus::Ready { version } => {
                            if ui.button(format!("再起動して {version} を適用")).clicked() {
                                self.restart_to_apply_update(ctx);
                            }
                        }
                        UpdateStatus::UpToDate => {
                            ui.label("最新バージョンです");
                            if ui.button("Check for Updates").clicked() {
                                updater::spawn_check(self.update_status.clone(), ctx.clone());
                            }
                        }
                        UpdateStatus::Failed(e) => {
                            ui.label(
                                egui::RichText::new("更新の確認/適用に失敗")
                                    .color(ui.visuals().warn_fg_color),
                            )
                            .on_hover_text(e);
                            if ui.button("Check for Updates").clicked() {
                                updater::spawn_check(self.update_status.clone(), ctx.clone());
                            }
                        }
                        UpdateStatus::Idle => {
                            if ui.button("Check for Updates").clicked() {
                                updater::spawn_check(self.update_status.clone(), ctx.clone());
                            }
                        }
                    }
                    ui.separator();
                    ui.hyperlink_to(
                        "GitHub Releases",
                        format!(
                            "https://github.com/{}/{}/releases",
                            updater::REPO_OWNER,
                            updater::REPO_NAME
                        ),
                    );
                });

                // 右端: 更新が利用可能／適用済みのときだけ目立つボタンを出す
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let status = self.update_status.lock().unwrap().clone();
                    match status {
                        UpdateStatus::Available { version, url } => {
                            let text = egui::RichText::new(format!("⬆ Update {version}"))
                                .color(Color32::from_rgb(255, 210, 90));
                            if ui.button(text).clicked() {
                                self.start_update(version, url, ctx);
                            }
                        }
                        UpdateStatus::Downloading { .. } => {
                            ui.spinner();
                        }
                        UpdateStatus::Ready { .. } => {
                            let text = egui::RichText::new("↻ 再起動して更新を適用")
                                .color(Color32::from_rgb(140, 220, 140));
                            if ui.button(text).clicked() {
                                self.restart_to_apply_update(ctx);
                            }
                        }
                        _ => {}
                    }
                });
            });
        });

        // 更新の適用（exe差し替え）が完了したら、一度だけ再起動を促す
        let ready_version = match &*self.update_status.lock().unwrap() {
            UpdateStatus::Ready { version } => Some(version.clone()),
            _ => None,
        };
        if let Some(version) = ready_version {
            if !self.restart_prompted {
                self.restart_prompted = true;
                let answer = rfd::MessageDialog::new()
                    .set_title("アップデート")
                    .set_description(&format!(
                        "バージョン {version} を適用しました。今すぐ再起動しますか？"
                    ))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show();
                if answer == rfd::MessageDialogResult::Yes {
                    self.restart_to_apply_update(ctx);
                }
            }
        }

        ctx.set_visuals(egui::Visuals::dark());

        egui::CentralPanel::default()
            // 既定フレームの内側余白をなくし、画像領域をパネル全体に広げる
            .frame(egui::Frame::default())
            .show(ctx, |ui| {
                let panel_rect = ui.available_rect_before_wrap();

                // fit モード: ウィンドウサイズが変わったら再フィット
                if self.config.initial_display_mode == "fit"
                    && self.last_available_size.map_or(true, |last| last != panel_rect.size())
                {
                    self.fit_to_screen(panel_rect.size());
                    self.last_available_size = Some(panel_rect.size());
                }

                // 画像の有無に関わらず有効なキー
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui.input(|i| i.key_pressed(Key::F11)) {
                    let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
                }
                if ui.input(|i| i.key_pressed(Key::O)) {
                    self.open_file_dialog(ctx);
                }

                let response = ui.interact(
                    panel_rect,
                    ui.id().with("drag_area"),
                    egui::Sense::click_and_drag(),
                );

                if self.current_image.is_some() {
                    // ---- フォルダ内ナビゲーション ----
                    if ui.input(|i| {
                        i.key_pressed(Key::ArrowRight)
                            || i.key_pressed(Key::PageDown)
                            || i.key_pressed(Key::Space)
                    }) {
                        self.load_adjacent_image(ctx, true);
                    } else if ui.input(|i| {
                        i.key_pressed(Key::ArrowLeft)
                            || i.key_pressed(Key::PageUp)
                            || i.key_pressed(Key::Backspace)
                    }) {
                        self.load_adjacent_image(ctx, false);
                    } else if ui.input(|i| i.key_pressed(Key::Home)) {
                        self.load_image_at(ctx, 0);
                    } else if ui.input(|i| i.key_pressed(Key::End)) {
                        self.load_image_at(ctx, self.image_paths.len().saturating_sub(1));
                    }

                    // マウスジェスチャー（右ドラッグ）で前後送り
                    let mouse_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(Pos2::ZERO);
                    let right_button = ui.input(|i| i.pointer.secondary_down());
                    if let Some(action) = self.mouse_gesture.update(mouse_pos.to_vec2(), right_button) {
                        match action.as_str() {
                            "<<<" => self.load_adjacent_image(ctx, false),
                            ">>>" => self.load_adjacent_image(ctx, true),
                            _ => {}
                        }
                    }

                    // ---- 回転（R=右90°, L=左90°）----
                    let rot_before = self.rotation;
                    if ui.input(|i| i.key_pressed(Key::R)) {
                        self.rotation = (self.rotation + 1) % 4;
                    }
                    if ui.input(|i| i.key_pressed(Key::L)) {
                        self.rotation = (self.rotation + 3) % 4;
                    }
                    if self.rotation != rot_before {
                        self.pan_offset = Vec2::ZERO;
                        if self.config.initial_display_mode == "fit" {
                            self.fit_to_screen(panel_rect.size());
                        }
                    }

                    // ---- 表示リセット ----
                    if ui.input(|i| i.key_pressed(Key::F)) {
                        // Fキー：位置リセット＆フィットウィンドウ表示
                        self.pan_offset = Vec2::ZERO;
                        self.fit_to_screen(panel_rect.size());
                    }
                    if ui.input(|i| i.key_pressed(Key::Num0)) {
                        // 0キー：位置リセット＆100%表示（scale = 1.0）
                        self.pan_offset = Vec2::ZERO;
                        self.scale = 1.0;
                    }

                    // ナビゲーション等でこのフレーム中に画像が読み込み直された場合、
                    // ここでフィットさせて「1フレームだけ等倍表示される」ちらつきを防ぐ
                    if self.config.initial_display_mode == "fit" && self.last_available_size.is_none() {
                        self.fit_to_screen(panel_rect.size());
                        self.last_available_size = Some(panel_rect.size());
                    }

                    // ---- ズーム（回転を考慮した表示サイズを基準に、アンカー位置固定で計算）----
                    let base_size = self.display_base_size();
                    if ui.input(|i| i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals)) {
                        let new_scale = (self.scale * KEY_ZOOM_STEP).clamp(MIN_SCALE, MAX_SCALE);
                        self.zoom_at(panel_rect.center().to_vec2(), &panel_rect, base_size, new_scale);
                    }
                    if ui.input(|i| i.key_pressed(Key::Minus)) {
                        let new_scale = (self.scale / KEY_ZOOM_STEP).clamp(MIN_SCALE, MAX_SCALE);
                        self.zoom_at(panel_rect.center().to_vec2(), &panel_rect, base_size, new_scale);
                    }

                    let wheel_delta = ui.input(|i| i.raw_scroll_delta.y);
                    if wheel_delta != 0.0 {
                        let factor = 1.0 + wheel_delta * self.config.wheel_zoom_factor;
                        let new_scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
                        // マウスカーソル位置を基準に拡大縮小
                        let anchor = response
                            .hover_pos()
                            .map(|p| p.to_vec2())
                            .unwrap_or_else(|| panel_rect.center().to_vec2());
                        self.zoom_at(anchor, &panel_rect, base_size, new_scale);
                    }

                    // ダブルクリックで フィット⇔100% をトグル
                    if response.double_clicked() {
                        self.pan_offset = Vec2::ZERO;
                        if (self.scale - 1.0).abs() < 0.01 {
                            self.fit_to_screen(panel_rect.size());
                        } else {
                            self.scale = 1.0;
                        }
                    }

                    // 左ドラッグでパン（右ドラッグはジェスチャー）
                    if response.dragged() && !ui.input(|i| i.pointer.secondary_down()) {
                        self.pan_offset += response.drag_delta();
                    }
                }

                // ---- 描画 ----
                self.draw_checker_background(ui);

                let scale = self.scale;
                let rotation = self.rotation;
                let pan = self.pan_offset;
                let base_size = self.display_base_size();
                let ppp = ctx.pixels_per_point();
                let max_dim = ctx.input(|i| i.max_texture_side).max(1) as f32;

                if let Some(image) = &mut self.current_image {
                    let scaled_size = base_size * scale;
                    let origin = panel_rect.min + (panel_rect.size() - scaled_size) * 0.5 + pan;
                    let image_rect = Rect::from_min_size(origin, scaled_size);

                    match image {
                        LoadedImage::Raster { texture, .. } => {
                            draw_texture_rotated(ui.painter(), texture, image_rect, rotation);
                        }
                        LoadedImage::Svg {
                            size,
                            texture,
                            view,
                            job_tx,
                            result_rx,
                            last_requested,
                            ..
                        } => {
                            // SVG はベクターなので、可視領域（＋余白）だけを表示解像度ちょうどで
                            // ラスタライズする（どんな拡大率でも 1px=1texel の鮮明さ）。
                            // ラスタライズはワーカースレッドで行い、UI はブロックしない。
                            // 完成までは手持ちのテクスチャを引き伸ばして表示する
                            // （精密な SVG ではズーム中に一瞬ぼやけ、止まると鮮明になる）。

                            // ワーカーからの完成テクスチャを受け取る（最後の 1 件だけ反映すれば十分）
                            let mut arrived: Option<SvgRenderResult> = None;
                            while let Ok(res) = result_rx.try_recv() {
                                arrived = Some(res);
                            }
                            if let Some(res) = arrived {
                                match texture.as_mut() {
                                    Some(t) => t.set(res.image, egui::TextureOptions::LINEAR),
                                    None => {
                                        *texture = Some(ctx.load_texture(
                                            "svg_view",
                                            res.image,
                                            egui::TextureOptions::LINEAR,
                                        ))
                                    }
                                }
                                *view = Some(res.view);
                            }

                            let scale_px = scale * ppp;
                            let visible = image_rect.intersect(panel_rect);
                            if visible.is_positive() && scale_px > 0.0 {
                                let (full_w, full_h) = rotated_dims(size[0], size[1], rotation);
                                let full_w_px = full_w * scale_px;
                                let full_h_px = full_h * scale_px;
                                // 可視部分（画像原点基準の物理px）
                                let nx0 = ((visible.min.x - image_rect.min.x) * ppp)
                                    .floor()
                                    .clamp(0.0, full_w_px);
                                let ny0 = ((visible.min.y - image_rect.min.y) * ppp)
                                    .floor()
                                    .clamp(0.0, full_h_px);
                                let nx1 = ((visible.max.x - image_rect.min.x) * ppp)
                                    .ceil()
                                    .clamp(0.0, full_w_px);
                                let ny1 = ((visible.max.y - image_rect.min.y) * ppp)
                                    .ceil()
                                    .clamp(0.0, full_h_px);

                                // このフレームで必要な crop（入力から決定的に計算される）
                                let (tx, tw) =
                                    crop_axis(nx0, nx1, full_w_px, SVG_RENDER_MARGIN_PX, max_dim);
                                let (ty, th) =
                                    crop_axis(ny0, ny1, full_h_px, SVG_RENDER_MARGIN_PX, max_dim);
                                let target = SvgView {
                                    scale_px,
                                    rot: rotation,
                                    crop: [tx, ty, tw, th],
                                };

                                // いまのテクスチャで十分か: スケール・回転が一致し、必要領域を
                                // 丸ごと含む（パン余白内）か、必要 crop と一致（可視領域が GPU 上限を
                                // 超えるケース）していれば追加のラスタライズは不要。
                                let satisfied = texture.is_some()
                                    && view.as_ref().map_or(false, |v| {
                                        v.rot == rotation
                                            && (v.scale_px - scale_px).abs() <= scale_px * 1e-4
                                            && (v.crop == target.crop
                                                || (v.crop[0] as f32 <= nx0
                                                    && v.crop[1] as f32 <= ny0
                                                    && (v.crop[0] + v.crop[2]) as f32 >= nx1
                                                    && (v.crop[1] + v.crop[3]) as f32 >= ny1))
                                    });

                                // 足りなければワーカーに依頼（同一要求の重複送信はしない）
                                if !satisfied && *last_requested != Some(target) {
                                    let svg_crop = map_display_crop_to_svg(
                                        rotation,
                                        size[0] * scale_px,
                                        size[1] * scale_px,
                                        [
                                            target.crop[0] as f32,
                                            target.crop[1] as f32,
                                            target.crop[2] as f32,
                                            target.crop[3] as f32,
                                        ],
                                    );
                                    let job = SvgRenderJob {
                                        scale_px,
                                        rot: rotation,
                                        crop: target.crop,
                                        svg_crop,
                                    };
                                    if job_tx.send(job).is_ok() {
                                        *last_requested = Some(target);
                                    }
                                }

                                // 描画: テクスチャが保持する領域を現在のビューへ写像して描く。
                                // スケールが一致していれば 1px=1texel の等倍描画。新しい結果が
                                // まだ届いていない間は旧テクスチャが引き伸ばされる（ボケるが固まらない）。
                                let mut drawn = false;
                                if let (Some(t), Some(v)) = (texture.as_ref(), view.as_ref()) {
                                    if v.rot == rotation && v.scale_px > 0.0 {
                                        let factor = scale_px / v.scale_px;
                                        let tex_rect = Rect::from_min_size(
                                            image_rect.min
                                                + egui::vec2(v.crop[0] as f32, v.crop[1] as f32)
                                                    * factor
                                                    / ppp,
                                            egui::vec2(v.crop[2] as f32, v.crop[3] as f32) * factor
                                                / ppp,
                                        );
                                        draw_texture_rotated(ui.painter(), t, tex_rect, v.rot);
                                        drawn = true;
                                    }
                                }

                                if !satisfied {
                                    // レンダリング待ちを示すスピナー（未描画なら中央、描画済みなら右上に小さく）
                                    let spinner_rect = if drawn {
                                        Rect::from_center_size(
                                            panel_rect.right_top() + egui::vec2(-24.0, 24.0),
                                            Vec2::splat(18.0),
                                        )
                                    } else {
                                        Rect::from_center_size(panel_rect.center(), Vec2::splat(32.0))
                                    };
                                    ui.put(spinner_rect, egui::Spinner::new());
                                    // 結果の取りこぼし防止の保険（通常はワーカーが repaint を要求する）
                                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                                }
                            }
                        }
                    }
                }

                // マウスジェスチャーの描画
                if self.mouse_gesture.is_active {
                    self.mouse_gesture.draw(ui, panel_rect.center());
                }
            });

        // タイトルバーに [位置/総数] サイズ 回転 拡大率 ファイルパスを表示（変化時のみ送信）
        let title = if let Some(image) = &self.current_image {
            let path = match image {
                LoadedImage::Raster { path, .. } => path,
                LoadedImage::Svg { path, .. } => path,
            };
            let pos_str = self
                .current_path
                .as_ref()
                .and_then(|p| self.image_paths.iter().position(|x| x == p))
                .map(|i| format!("[{}/{}] ", i + 1, self.image_paths.len()))
                .unwrap_or_default();
            let dims = self
                .image_size
                .map(|s| format!("{}x{} ", s[0], s[1]))
                .unwrap_or_default();
            let rot = match self.rotation {
                1 => "90° ",
                2 => "180° ",
                3 => "270° ",
                _ => "",
            };
            format!(
                "MSBT-yuina - {}{}{}{}% - {}",
                pos_str,
                dims,
                rot,
                (self.scale * 100.0).round() as i32,
                path.display()
            )
        } else {
            "MSBT-yuina".to_string()
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    /// チェッカーボード風の背景を描画
    fn draw_checker_background(&self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let painter = ui.painter();
        let dark = Color32::from_gray(32);
        let bright = Color32::from_gray(64);
        let size = 16.0;
        let mut y = 0.0;
        while y * size <= rect.height() {
            let mut x = 0.0;
            while x * size <= rect.width() {
                let color = if (y as i32 + x as i32) % 2 == 0 { dark } else { bright };
                let cell_rect = Rect::from_min_size(
                    rect.min + Vec2::new(x * size, y * size),
                    Vec2::splat(size),
                );
                painter.rect_filled(cell_rect, 0.0, color);
                x += 1.0;
            }
            y += 1.0;
        }
    }
}

impl eframe::App for ImageViewer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update(ctx);
    }
}

/// 画像を Windows の WIC（OS が持つ画像コーデック）でデコードして RGBA 画像を返す。
/// image クレートが非対応の形式（HEIC/HEIF/AVIF/JPEG XR/カメラRAW 等）のフォールバックに使う。
/// 形式によっては Microsoft Store の拡張機能（例: HEIF 画像拡張機能 / AV1 ビデオ拡張機能）が必要。
#[cfg(windows)]
fn decode_via_wic(path: &Path) -> Result<image::DynamicImage, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory,
        WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        // winit が COM を初期化済みのはずだが念のため。初期化済み／モード差異のエラーは無視。
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("WIC ファクトリの生成に失敗: {e}"))?;

        let decoder = factory
            .CreateDecoderFromFilename(
                PCWSTR(wide.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(|e| {
                format!("この形式をデコードできません（対応コーデック/拡張機能が未導入の可能性）: {e}")
            })?;

        let frame = decoder
            .GetFrame(0)
            .map_err(|e| format!("フレーム取得に失敗: {e}"))?;

        // どのピクセル形式の HEIC でも 32bpp RGBA に変換してから取り出す。
        let converter = factory
            .CreateFormatConverter()
            .map_err(|e| format!("フォーマット変換器の生成に失敗: {e}"))?;
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppRGBA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(|e| format!("RGBA への変換に失敗: {e}"))?;

        let mut w: u32 = 0;
        let mut h: u32 = 0;
        converter
            .GetSize(&mut w, &mut h)
            .map_err(|e| format!("画像サイズの取得に失敗: {e}"))?;
        if w == 0 || h == 0 {
            return Err("画像サイズが不正です".into());
        }

        let stride = w.checked_mul(4).ok_or("画像が大きすぎます")?;
        let buf_len = (stride as usize)
            .checked_mul(h as usize)
            .ok_or("画像が大きすぎます")?;
        let mut buf = vec![0u8; buf_len];
        converter
            .CopyPixels(std::ptr::null(), stride, &mut buf)
            .map_err(|e| format!("ピクセルの取得に失敗: {e}"))?;

        let img = image::RgbaImage::from_raw(w, h, buf).ok_or("バッファサイズが不一致")?;
        Ok(image::DynamicImage::ImageRgba8(img))
    }
}

/// 非 Windows では WIC フォールバック非対応（このアプリは Windows 専用だが、cfg を明示しておく）。
#[cfg(not(windows))]
fn decode_via_wic(_path: &Path) -> Result<image::DynamicImage, String> {
    Err("この形式は Windows でのみ対応しています".into())
}

fn create_fallback_icon() -> IconData {
    IconData {
        rgba: vec![0, 0, 0, 0],
        width: 1,
        height: 1,
    }
}

fn load_icon() -> IconData {
    info!("アイコンの読み込み開始");
    let icon_result = || -> Result<IconData, Box<dyn std::error::Error>> {
        let exe_path = std::env::current_exe()?;
        let exe_dir = exe_path.parent().ok_or("Failed to get executable directory")?;
        let icon_path = exe_dir.join("icon.ico");
        if !icon_path.exists() {
            info!("アイコンファイルが見つかりません: {:?}", icon_path);
            return Ok(create_fallback_icon());
        }
        let icon_data = fs::read(&icon_path)?;
        let icon = ico::IconDir::read(Cursor::new(icon_data))?;
        if icon.entries().is_empty() {
            info!("アイコンファイルにエントリがありません");
            return Ok(create_fallback_icon());
        }
        let target_size = 32;
        let entry = icon.entries().iter()
            .min_by_key(|e| {
                let size = e.width() as i32;
                (size - target_size).abs()
            })
            .ok_or("No suitable icon found")?;
        let icon_image = entry.decode()?;
        let width = entry.width() as u32;
        let height = entry.height() as u32;
        let rgba: Vec<u8> = icon_image.rgba_data().to_vec();
        info!("アイコンの読み込み完了: {}x{} pixels", width, height);
        Ok(IconData { rgba, width, height })
    }();
    match icon_result {
        Ok(icon) => icon,
        Err(e) => {
            error!("アイコンの読み込みに失敗: {}", e);
            create_fallback_icon()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_cmp_orders_numeric_runs() {
        assert_eq!(natural_cmp("img2.png", "img10.png"), Ordering::Less);
        assert_eq!(natural_cmp("img10.png", "img2.png"), Ordering::Greater);
        assert_eq!(natural_cmp("a.png", "b.png"), Ordering::Less);
        assert_eq!(natural_cmp("IMG5.png", "img5.png"), Ordering::Equal.then(Ordering::Equal));
        // 大文字小文字を無視して等価に扱われる（数値部の後まで一致）
        assert_eq!(natural_cmp("IMG5.PNG", "img5.png"), Ordering::Equal);
        assert_eq!(natural_cmp("img05.png", "img5.png"), Ordering::Greater); // 同値なら元の桁数で安定化
        assert_eq!(natural_cmp("2.png", "10.png"), Ordering::Less);
        assert_eq!(natural_cmp("", "a"), Ordering::Less);
    }

    #[test]
    fn rotated_dims_swaps_on_odd_rotations() {
        assert_eq!(rotated_dims(100.0, 50.0, 0), (100.0, 50.0));
        assert_eq!(rotated_dims(100.0, 50.0, 1), (50.0, 100.0));
        assert_eq!(rotated_dims(100.0, 50.0, 2), (100.0, 50.0));
        assert_eq!(rotated_dims(100.0, 50.0, 3), (50.0, 100.0));
    }

    #[test]
    fn crop_mapping_full_rect_is_identity() {
        let (ws, hs) = (100.0, 50.0);
        for rot in 0..4u8 {
            let (dw, dh) = rotated_dims(ws, hs, rot);
            let mapped = map_display_crop_to_svg(rot, ws, hs, [0.0, 0.0, dw, dh]);
            assert_eq!(mapped, [0.0, 0.0, ws, hs], "rot={rot}");
        }
    }

    #[test]
    fn crop_mapping_corners() {
        let (ws, hs) = (100.0, 50.0);
        // rot=1（時計回り90°）: 表示の左上の小片は、SVG 空間では左下
        assert_eq!(
            map_display_crop_to_svg(1, ws, hs, [0.0, 0.0, 10.0, 20.0]),
            [0.0, 40.0, 20.0, 10.0]
        );
        // rot=2（180°）: 表示の左上は SVG の右下
        assert_eq!(
            map_display_crop_to_svg(2, ws, hs, [0.0, 0.0, 10.0, 20.0]),
            [90.0, 30.0, 10.0, 20.0]
        );
        // rot=3（270°）: 表示の左上は SVG の右上
        assert_eq!(
            map_display_crop_to_svg(3, ws, hs, [0.0, 0.0, 10.0, 20.0]),
            [80.0, 0.0, 20.0, 10.0]
        );
    }

    #[test]
    fn crop_axis_covers_visible_and_respects_max() {
        // 通常ケース: 可視区間＋余白、画像端でクランプ
        assert_eq!(crop_axis(100.0, 500.0, 10000.0, 256.0, 8192.0), (0, 756));
        assert_eq!(crop_axis(9900.0, 10000.0, 10000.0, 256.0, 8192.0), (9644, 356));
        // 可視区間が GPU 上限を超える場合: 中央の max_dim 窓になり、入力が同じなら結果も同じ
        let a = crop_axis(0.0, 3000.0, 5000.0, 256.0, 2048.0);
        assert_eq!(a, crop_axis(0.0, 3000.0, 5000.0, 256.0, 2048.0));
        assert_eq!(a.1, 2048);
        let center = 1500u32;
        assert!(a.0 <= center && center <= a.0 + a.1, "窓が可視中央を含まない: {a:?}");
        // 余白を含めても max_dim を超えず、可視区間は丸ごと含む
        let b = crop_axis(1000.0, 2900.0, 5000.0, 256.0, 2048.0);
        assert!(b.1 <= 2048);
        assert!(b.0 as f32 <= 1000.0 && (b.0 + b.1) as f32 >= 2900.0, "{b:?}");
    }

    #[test]
    fn decompress_if_gzip_roundtrip() {
        use std::io::Write as _;
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(svg).unwrap();
        let gz = enc.finish().unwrap();
        assert_eq!(decompress_if_gzip(gz).unwrap(), svg.to_vec());
        // 非圧縮データはそのまま
        assert_eq!(decompress_if_gzip(svg.to_vec()).unwrap(), svg.to_vec());
    }

    #[test]
    fn font_magic_detection() {
        assert!(is_supported_font(b"\x00\x01\x00\x00rest"));
        assert!(is_supported_font(b"OTTOrest"));
        assert!(!is_supported_font(b"wOFFrest"));
        assert!(!is_supported_font(b"wOF2rest"));
        assert!(!is_supported_font(b"<sv"));
    }

    fn render_svg(svg: &str, db: usvg::fontdb::Database, w: u32, h: u32) -> Pixmap {
        let mut opt = Options::default();
        opt.fontdb = Arc::new(db);
        let tree = Tree::from_str(svg, &opt).expect("SVG parse failed");
        let mut pixmap = Pixmap::new(w, h).unwrap();
        resvg::render(&tree, usvg::Transform::default(), &mut pixmap.as_mut());
        pixmap
    }

    /// y ∈ [y0, y1) の帯にある不透過ピクセル数
    fn opaque_pixels_in_band(pixmap: &Pixmap, y0: u32, y1: u32) -> usize {
        let w = pixmap.width();
        pixmap
            .pixels()
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                let y = (*i as u32) / w;
                y >= y0 && y < y1 && p.alpha() > 0
            })
            .count()
    }

    #[test]
    fn svg_text_renders_with_system_fonts() {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        assert!(db.len() > 0, "システムフォントが1つも見つからない");
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="100">
            <text x="10" y="40" font-family="sans-serif" font-size="36" fill="black">Hello</text>
            <text x="10" y="90" font-family="sans-serif" font-size="36" fill="black">こんにちは漢字</text>
        </svg>"#;
        let pixmap = render_svg(svg, db, 400, 100);
        assert!(
            opaque_pixels_in_band(&pixmap, 0, 50) > 50,
            "ラテン文字のテキストが描画されていない"
        );
        assert!(
            opaque_pixels_in_band(&pixmap, 50, 100) > 50,
            "日本語テキストが描画されていない（フォントフォールバック不全）"
        );
    }

    #[test]
    fn embedded_font_face_is_loaded_and_rendered() {
        use base64::Engine as _;
        // システムフォントを一切ロードしない空のDBに、@font-face の data URI だけで
        // フォントが供給されることを確認する（実フォントとして Windows の Arial を利用）
        let font_path = Path::new("C:/Windows/Fonts/arial.ttf");
        if !font_path.exists() {
            eprintln!("skip: {} が見つからないためスキップ", font_path.display());
            return;
        }
        let font_bytes = fs::read(font_path).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&font_bytes);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="60">
              <style>
                @font-face {{
                  font-family: 'Arial';
                  src: url(data:font/ttf;base64,{b64});
                }}
              </style>
              <text x="10" y="45" font-family="Arial" font-size="40" fill="black">Embedded</text>
            </svg>"#
        );

        // ローダー無しでは 1 フォントも無く、テキストは描画されない
        let empty_db = usvg::fontdb::Database::new();
        let blank = render_svg(&svg, empty_db, 400, 60);
        assert_eq!(opaque_pixels_in_band(&blank, 0, 60), 0, "空DBで描画されるのは想定外");

        // ローダーを通すと描画される
        let mut db = usvg::fontdb::Database::new();
        let n = load_embedded_fonts(&svg, &mut db, None);
        assert_eq!(n, 1, "@font-face のフォントを読み込めていない");
        let pixmap = render_svg(&svg, db, 400, 60);
        assert!(
            opaque_pixels_in_band(&pixmap, 0, 60) > 100,
            "埋め込みフォントのテキストが描画されていない"
        );
    }

    #[test]
    fn tree_is_flat_detects_group_effects() {
        let opt = Options::default();
        let flat = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <g transform="translate(10,10)"><circle cx="10" cy="10" r="5" fill="red"/></g>
            <rect x="50" y="50" width="20" height="20" fill="blue"/>
        </svg>"#;
        let tree = Tree::from_str(flat, &opt).unwrap();
        assert!(tree_is_flat(tree.root()), "transform だけのグループはフラット扱いのはず");

        let not_flat = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <g opacity="0.5"><circle cx="10" cy="10" r="5" fill="red"/></g>
        </svg>"#;
        let tree = Tree::from_str(not_flat, &opt).unwrap();
        assert!(!tree_is_flat(tree.root()), "opacity 付きグループはフラットではない");
    }

    /// カリング描画が全体描画と同じ絵を出すこと（クロップ領域をピクセル比較）
    #[test]
    fn culled_render_matches_full_render() {
        let opt = Options::default();
        // 円・矩形・線を散らしたフラットな SVG（クロップ境界をまたぐ要素も含む）
        let mut svg = String::from(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="400">"#,
        );
        let mut seed = 12345u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) % 400) as f32
        };
        for _ in 0..300 {
            let (x, y, r) = (next(), next(), 3.0 + next() % 20.0);
            svg.push_str(&format!(
                r##"<circle cx="{x}" cy="{y}" r="{r}" fill="none" stroke="#c33" stroke-width="1.5"/>"##
            ));
        }
        for _ in 0..100 {
            let (x, y) = (next(), next());
            svg.push_str(&format!(
                r##"<rect x="{x}" y="{y}" width="30" height="18" fill="#356"/>"##
            ));
        }
        // 祖先グループの変換が正しく合成されることも検証する（入れ子 transform）
        svg.push_str(
            r##"<g transform="translate(40,20)"><g transform="scale(1.5)">
                <circle cx="60" cy="70" r="25" fill="none" stroke="#181" stroke-width="2"/>
                <rect x="30" y="90" width="40" height="22" fill="#815"/>
            </g></g>"##,
        );
        svg.push_str("</svg>");
        let tree = Tree::from_str(&svg, &opt).unwrap();
        assert!(tree_is_flat(tree.root()));

        // 同一クロップ [x=100, y=120, w=200, h=180]（物理px）を
        // 「木全体の描画」と「カリング描画」の両方で描いて比較する。
        // （ピクスマップ境界1px のAA近似はクロップ描画自体の性質で両者共通なので、
        //  この比較ならカリングによる差だけが検出できる）
        let scale = 2.0f32;
        let (cx, cy, cw, ch) = (100u32, 120u32, 200u32, 180u32);
        let ts = usvg::Transform::from_scale(scale, scale)
            .post_translate(-(cx as f32), -(cy as f32));
        let mut full = Pixmap::new(cw, ch).unwrap();
        resvg::render(&tree, ts, &mut full.as_mut());

        let mut part = Pixmap::new(cw, ch).unwrap();
        let pad = 2.0 / scale;
        let clip = tiny_skia::Rect::from_xywh(
            cx as f32 / scale - pad,
            cy as f32 / scale - pad,
            cw as f32 / scale + pad * 2.0,
            ch as f32 / scale + pad * 2.0,
        )
        .unwrap();
        let (mut drawn, mut culled) = (0u32, 0u32);
        render_culled(tree.root(), clip, ts, &mut part.as_mut(), &mut drawn, &mut culled);
        assert!(drawn > 0, "何も描画されていない");
        assert!(culled > 0, "何もカリングされていない（テストの意味がない）");

        // ピクセル比較（浮動小数の丸め差を考慮し、チャンネル差 1 まで許容）
        let mut mismatches = 0usize;
        let mut max_diff = 0u8;
        let mut samples: Vec<(u32, u32, [u8; 4], [u8; 4])> = Vec::new();
        for y in 0..ch {
            for x in 0..cw {
                let a = part.pixel(x, y).unwrap();
                let b = full.pixel(x, y).unwrap();
                let d = a
                    .red()
                    .abs_diff(b.red())
                    .max(a.green().abs_diff(b.green()))
                    .max(a.blue().abs_diff(b.blue()))
                    .max(a.alpha().abs_diff(b.alpha()));
                if d > 1 {
                    mismatches += 1;
                    max_diff = max_diff.max(d);
                    if samples.len() < 8 {
                        samples.push((
                            x,
                            y,
                            [a.red(), a.green(), a.blue(), a.alpha()],
                            [b.red(), b.green(), b.blue(), b.alpha()],
                        ));
                    }
                }
            }
        }
        assert_eq!(
            mismatches, 0,
            "カリング描画と全体描画の絵が一致しない（{drawn} 描画 / {culled} カリング, 最大差 {max_diff}, 例 {samples:?}）"
        );
    }

    /// 40,000 個のストローク円を散らした精密 SVG（性能テスト用）
    fn build_heavy_svg() -> String {
        let mut svg = String::from(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="2000" height="2000">"#,
        );
        let mut seed = 42u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) % 2000) as f32
        };
        for _ in 0..40000 {
            let (x, y, r) = (next(), next(), 2.0 + next() % 12.0);
            svg.push_str(&format!(
                r##"<circle cx="{x}" cy="{y}" r="{r}" fill="none" stroke="#357" stroke-width="0.6"/>"##
            ));
        }
        svg.push_str("</svg>");
        svg
    }

    /// GPU 描画が CPU 描画とおおよそ同じ絵を出すこと（被覆率で比較。
    /// ラスタライザが違うため AA は完全一致しない）。GPU 必須のため ignored。
    #[test]
    #[ignore]
    fn live_gpu_render_matches_cpu_roughly() {
        let opt = Options::default();
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <circle cx="30" cy="30" r="20" fill="#c33"/>
            <rect x="50" y="55" width="40" height="30" fill="none" stroke="#357" stroke-width="3"/>
            <path d="M 10 80 Q 50 40 90 80" fill="none" stroke="#181" stroke-width="2"/>
        </svg>"##;
        let tree = Tree::from_str(svg, &opt).unwrap();
        assert!(tree_is_flat(tree.root()));
        let mut gpu = match GpuRenderer::new() {
            Ok(g) => g,
            Err(e) => {
                eprintln!("skip: GPU が使えません: {e}");
                return;
            }
        };
        let job = SvgRenderJob {
            scale_px: 2.0,
            rot: 0,
            crop: [0, 0, 200, 200],
            svg_crop: [0.0, 0.0, 200.0, 200.0],
        };
        let (img, _, _) = gpu.render(&tree, true, &job).expect("GPU描画に失敗");
        assert_eq!(img.size, [200, 200]);

        let mut pixmap = Pixmap::new(200, 200).unwrap();
        resvg::render(
            &tree,
            usvg::Transform::from_scale(2.0, 2.0),
            &mut pixmap.as_mut(),
        );
        let gpu_opaque = img.pixels.iter().filter(|p| p.a() > 8).count();
        let cpu_opaque = pixmap.pixels().iter().filter(|p| p.alpha() > 8).count();
        println!("被覆率: GPU {gpu_opaque} px / CPU {cpu_opaque} px");
        assert!(gpu_opaque > 500, "GPU出力がほぼ空: {gpu_opaque}");
        let ratio = gpu_opaque as f64 / cpu_opaque.max(1) as f64;
        assert!(
            (0.85..=1.15).contains(&ratio),
            "GPU/CPU の被覆率が乖離: {ratio:.3}"
        );
    }

    /// GPU カリングにより、どの倍率・位置でも出力が空にならないことの実機確認。
    /// （カリング無しの全体シーン方式では、高倍率で vello の内部バッファが溢れて
    /// 全ケース空になることを確認済み — それが gpu_usable_now ガードの理由）
    #[test]
    #[ignore]
    fn live_gpu_culled_zoom_not_blank() {
        let opt = Options::default();
        let tree = Tree::from_str(&build_heavy_svg(), &opt).unwrap();
        assert!(tree_is_flat(tree.root()));
        let mut gpu = match GpuRenderer::new() {
            Ok(g) => g,
            Err(e) => {
                eprintln!("skip: {e}");
                return;
            }
        };
        let cases = [
            ("scale20 origin", 20.0f32, [0u32, 0, 1000, 1000]),
            ("scale20 t=15000", 20.0, [15000, 15000, 1000, 1000]),
            ("scale20 t=30000", 20.0, [30000, 30000, 1000, 1000]),
            ("scale5 t=3750", 5.0, [3750, 3750, 1000, 1000]),
            ("scale0.43 origin", 0.43, [0, 0, 860, 860]),
        ];
        for (label, scale, crop) in cases {
            let job = SvgRenderJob {
                scale_px: scale,
                rot: 0,
                crop,
                svg_crop: [crop[0] as f32, crop[1] as f32, crop[2] as f32, crop[3] as f32],
            };
            let (img, drawn, culled) = gpu.render(&tree, true, &job).expect(label);
            let opaque = img.pixels.iter().filter(|p| p.a() > 0).count();
            println!("{label}: 不透過 {opaque} px（描画 {drawn} / カリング {culled}）");
            assert!(opaque > 0, "{label} が空");
        }
    }

    /// GPU vs CPU の性能比較（cargo test --release perf_gpu -- --ignored --nocapture）
    #[test]
    #[ignore]
    fn perf_gpu_vs_cpu() {
        let opt = Options::default();
        let tree = Tree::from_str(&build_heavy_svg(), &opt).unwrap();
        let mut gpu = match GpuRenderer::new() {
            Ok(g) => g,
            Err(e) => {
                eprintln!("skip: GPU が使えません: {e}");
                return;
            }
        };
        // 全景（フィット相当: 0.43倍 ≒ 860x860）と拡大20倍の両方を測る
        let cases = [
            ("全景 0.43x", 0.43f32, [0u32, 0, 860, 860]),
            ("拡大 20x", 20.0f32, [15000u32, 15000, 1000, 1000]),
        ];
        for (label, scale, crop) in cases {
            let job = SvgRenderJob {
                scale_px: scale,
                rot: 0,
                crop,
                svg_crop: [crop[0] as f32, crop[1] as f32, crop[2] as f32, crop[3] as f32],
            };
            // 初回はシェーダコンパイル等があるためウォームアップ
            let _ = gpu.render(&tree, true, &job);
            let t0 = std::time::Instant::now();
            let (img, _, _) = gpu.render(&tree, true, &job).expect("GPU描画に失敗");
            let gpu_ms = t0.elapsed().as_millis();
            assert!(img.pixels.iter().any(|p| p.a() > 0));

            let mut pixmap = Pixmap::new(crop[2], crop[3]).unwrap();
            let ts = usvg::Transform::from_scale(scale, scale)
                .post_translate(-(crop[0] as f32), -(crop[1] as f32));
            let clip = tiny_skia::Rect::from_xywh(
                crop[0] as f32 / scale,
                crop[1] as f32 / scale,
                crop[2] as f32 / scale,
                crop[3] as f32 / scale,
            )
            .unwrap();
            let (mut drawn, mut culled) = (0u32, 0u32);
            let t1 = std::time::Instant::now();
            render_culled(tree.root(), clip, ts, &mut pixmap.as_mut(), &mut drawn, &mut culled);
            let cpu_ms = t1.elapsed().as_millis();
            println!(
                "{label}: GPU {gpu_ms} ms / CPU(カリング) {cpu_ms} ms（CPU側 描画 {drawn}, カリング {culled}）"
            );
        }
    }

    /// 性能計測（cargo test --release perf_culled -- --ignored --nocapture で実行）
    #[test]
    #[ignore]
    fn perf_culled_zoom_vs_full() {
        let opt = Options::default();
        let tree = Tree::from_str(&build_heavy_svg(), &opt).unwrap();
        assert!(tree_is_flat(tree.root()));

        // 拡大 20 倍で 1000x1000 の可視領域（画面相当）を描く
        let scale = 20.0f32;
        let (cx, cy, cw, ch) = (15000.0f32, 15000.0f32, 1000u32, 1000u32);
        let ts = usvg::Transform::from_scale(scale, scale).post_translate(-cx, -cy);
        let clip = tiny_skia::Rect::from_xywh(
            cx / scale,
            cy / scale,
            cw as f32 / scale,
            ch as f32 / scale,
        )
        .unwrap();

        let mut p1 = Pixmap::new(cw, ch).unwrap();
        let t0 = std::time::Instant::now();
        resvg::render(&tree, ts, &mut p1.as_mut());
        let full_ms = t0.elapsed().as_millis();

        let mut p2 = Pixmap::new(cw, ch).unwrap();
        let (mut drawn, mut culled) = (0u32, 0u32);
        let t1 = std::time::Instant::now();
        render_culled(tree.root(), clip, ts, &mut p2.as_mut(), &mut drawn, &mut culled);
        let culled_ms = t1.elapsed().as_millis();

        println!(
            "拡大20倍 1000x1000: 全体描画 {full_ms} ms / カリング描画 {culled_ms} ms（描画 {drawn}, カリング {culled}）"
        );
        assert!(culled_ms <= full_ms, "カリングが逆効果になっている");
    }

    #[test]
    fn embedded_font_ignores_woff_and_http() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
          <style>
            @font-face { font-family: A; src: url(https://example.com/font.ttf); }
            @font-face { font-family: B; src: url(data:font/woff2;base64,d09GMgABAAAA); }
          </style>
        </svg>"#;
        let mut db = usvg::fontdb::Database::new();
        assert_eq!(load_embedded_fonts(svg, &mut db, None), 0);
    }
}
