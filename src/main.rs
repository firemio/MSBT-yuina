#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, Key, Rect, Vec2, Pos2};
use egui::IconData;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use log::{error, info, LevelFilter};
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
};
use std::panic;
use ico;
use serde::{Deserialize, Serialize};
use tiny_skia::Pixmap;
use usvg::{Options, Tree};
use resvg;
use rfd;
use image;

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
}

fn default_wheel_zoom_factor() -> f32 {
    0.001
}

impl Default for ViewerConfig {
    fn default() -> Self {
        Self {
            initial_display_mode: "fit".to_string(),
            enable_debug_log: false,
            wheel_zoom_factor: default_wheel_zoom_factor(),
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
             wheel_zoom_factor = {}\n",
            self.initial_display_mode,
            self.enable_debug_log,
            self.wheel_zoom_factor
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
            Box::new(ImageViewer::new(cc, initial_image, config))
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
        follow_system_theme: true,
        default_theme: eframe::Theme::Dark,
        ..Default::default()
    };
    Ok(options)
}

/// 読み込んだ画像の種類を表す型  
/// Raster: 通常画像  
/// Svg: SVG の場合、usvg::Tree と元のサイズ、現在のテクスチャ、最後にレンダリングした scale を保持
enum LoadedImage {
    Raster {
        texture: egui::TextureHandle,
        path: PathBuf,
    },
    Svg {
        tree: Tree,
        original_size: [u32; 2],
        texture: egui::TextureHandle,
        last_scale: f32,
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

struct ImageViewer {
    config: ViewerConfig,
    current_image: Option<LoadedImage>,
    current_path: Option<PathBuf>,
    image_size: Option<[u32; 2]>,
    scale: f32,
    pan_offset: Vec2,
    image_paths: Vec<PathBuf>,
    // 前回の利用可能なウィンドウサイズ（"fit" モードで使用）
    last_available_size: Option<Vec2>,
    mouse_gesture: MouseGesture,
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
            image_paths: Vec::new(),
            last_available_size: None,
            mouse_gesture: MouseGesture::new(),
        };
        
        // 初期画像が指定されている場合は読み込み、かつディレクトリ内の画像一覧を更新する
        if let Some(path) = initial_image {
            if path.exists() {
                viewer.load_image(&path, &cc.egui_ctx);
                viewer.update_image_list(&path);
            } else {
                error!("指定された画像が見つかりません: {}", path.display());
            }
        }
        
        viewer
    }

    /// 画像サイズに合わせ、利用可能領域全体に収まる scale を計算する
    fn fit_to_screen(&mut self, ctx: &egui::Context) {
        if let Some(size) = self.image_size {
            let available_size = ctx.available_rect().size();
            let image_aspect = size[0] as f32 / size[1] as f32;
            let screen_aspect = available_size.x / available_size.y;
            self.scale = if image_aspect > screen_aspect {
                available_size.x / size[0] as f32
            } else {
                available_size.y / size[1] as f32
            };
            info!("画面に合わせてスケールを設定: {}", self.scale);
        }
    }

    /// 指定パスの画像を読み込み、拡大率、パン位置、画像サイズを更新する
    fn load_image(&mut self, path: &Path, ctx: &egui::Context) -> bool {
        info!("画像を読み込もうとしています: {:?}", path);
        self.pan_offset = Vec2::ZERO;
        self.scale = 1.0;
        self.image_size = None;

        let result = if let Some(ext) = path.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            if ext == "svg" {
                self.load_svg(path, ctx)
            } else {
                self.load_raster(path, ctx)
            }
        } else {
            let message = format!("サポートされていないファイル形式です: {}", path.display());
            error!("{}", message);
            rfd::MessageDialog::new()
                .set_title("エラー")
                .set_description(&message)
                .show();
            false
        };

        if result {
            self.current_path = Some(path.to_path_buf());
            // 画像の読み込みが成功したら、initial_display_modeに応じてスケールを設定
            if self.config.initial_display_mode == "fit" {
                self.fit_to_screen(ctx);
            }
        }

        result
    }

    fn load_svg(&mut self, path: &Path, ctx: &egui::Context) -> bool {
        if let Ok(svg_data) = fs::read_to_string(path) {
            info!("SVGファイルを読み込みました: {} bytes", svg_data.len());
            let opt = Options::default();
            if let Ok(tree) = Tree::from_str(&svg_data, &opt) {
                let size = tree.size();
                let width = size.width() as u32;
                let height = size.height() as u32;
                info!("SVGサイズ: {}x{}", width, height);
                self.image_size = Some([width, height]);

                // 初期レンダリング（scale=1.0）
                if let Some(mut pixmap) = Pixmap::new(width, height) {
                    resvg::render(&tree, usvg::Transform::default(), &mut pixmap.as_mut());
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        pixmap.data(),
                    );
                    let texture = ctx.load_texture(
                        path.to_string_lossy().to_string(),
                        image,
                        Default::default(),
                    );
                    self.current_image = Some(LoadedImage::Svg {
                        tree,
                        original_size: [width, height],
                        texture,
                        last_scale: 1.0,
                        path: path.to_path_buf(),
                    });
                    info!("SVGの読み込みが完了しました");
                    true
                } else {
                    let message = format!("SVGの描画に失敗しました: メモリが不足している可能性があります");
                    error!("{}", message);
                    rfd::MessageDialog::new()
                        .set_title("エラー")
                        .set_description(&message)
                        .show();
                    false
                }
            } else {
                let message = format!("SVGの解析に失敗しました: {}", path.display());
                error!("{}", message);
                rfd::MessageDialog::new()
                    .set_title("エラー")
                    .set_description(&message)
                    .show();
                false
            }
        } else {
            let message = format!("SVGファイルの読み込みに失敗しました: {}", path.display());
            error!("{}", message);
            rfd::MessageDialog::new()
                .set_title("エラー")
                .set_description(&message)
                .show();
            false
        }
    }

    fn load_raster(&mut self, path: &Path, ctx: &egui::Context) -> bool {
        match image::open(path) {
            Ok(image) => {
                let image = image.to_rgba8();
                let width = image.width() as usize;
                let height = image.height() as usize;
                self.image_size = Some([width as u32, height as u32]);
                if self.config.initial_display_mode == "fit" {
                    self.fit_to_screen(ctx);
                }
                let size = [width, height];
                let pixels = image.into_vec();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                let texture = ctx.load_texture(
                    path.to_string_lossy().to_string(),
                    color_image,
                    Default::default(),
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
                                        ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg"]
                                            .contains(&ext.as_str())
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
                    files.sort();
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
                    self.image_paths.len() - 1
                }
            };
            if let Some(path) = self.image_paths.get(new_index).cloned() {
                self.load_image(&path, ctx);
            }
        }
    }

    /// アプリケーション更新処理  
    /// ・ドラッグ＆ドロップによるファイル読み込み  
    /// ・メニューバー（File / Options）の表示  
    /// ・"fit" モードの場合、ウィンドウサイズ変更時に scale 再計算  
    /// ・SVG は、現在の scale と前回レンダリング時の scale の差が ±5%以上なら再レンダリングを実施  
    /// ・Fキーを押すと位置リセット＆フィットウィンドウ表示、0キーを押すと100%（scale=1.0）表示
    fn update(&mut self, ctx: &egui::Context) {
        // ドラッグ＆ドロップ対応
        let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
        for dropped in dropped_files {
            if let Some(path) = dropped.path {
                self.load_image(&path, ctx);
                self.update_image_list(&path);
            }
        }

        // メニューバー（File / Options）の表示
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open").clicked() {
                        if let Some(file_path) = rfd::FileDialog::new()
                            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp", "svg"])
                            .pick_file()
                        {
                            self.load_image(&file_path, ctx);
                            self.update_image_list(&file_path);
                        }
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
                        ui.label("Other Settings");
                        ui.separator();
                        ui.checkbox(&mut self.config.enable_debug_log, "Enable Debug Log");
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
            });
        });

        // 画像が未読込の場合、current_path があれば読み込み
        if self.current_image.is_none() {
            if let Some(path) = self.current_path.clone() {
                self.load_image(&path, ctx);
            }
        }

        ctx.set_visuals(egui::Visuals::dark());

        egui::CentralPanel::default().show(ctx, |ui| {
            let available_rect = ui.available_rect_before_wrap();
            if self.config.initial_display_mode == "fit" {
                if self.last_available_size.map_or(true, |last| last != available_rect.size()) {
                    self.fit_to_screen(ctx);
                    self.last_available_size = Some(available_rect.size());
                }
            }

            // SVG の場合、現在の scale と前回レンダリング時の scale に ±5%以上の差があれば再レンダリング
            if let Some(LoadedImage::Svg {
                tree,
                original_size,
                ref mut texture,
                ref mut last_scale,
                ..
            }) = &mut self.current_image
            {
                if self.scale > *last_scale * 1.05 || self.scale < *last_scale * 0.95 {
                    let render_width = (original_size[0] as f32 * self.scale).ceil() as u32;
                    let render_height = (original_size[1] as f32 * self.scale).ceil() as u32;
                    
                    info!("SVG再レンダリング: {}x{} (scale: {})", render_width, render_height, self.scale);
                    
                    if let Some(mut pixmap) = Pixmap::new(render_width, render_height) {
                        resvg::render(tree, usvg::Transform::from_scale(self.scale, self.scale), &mut pixmap.as_mut());
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [render_width as usize, render_height as usize],
                            pixmap.data(),
                        );
                        *texture = ctx.load_texture("svg_texture", image, Default::default());
                        *last_scale = self.scale;
                    }
                }
            }

            self.draw_checker_background(ui);

            if let Some(image) = &self.current_image {
                let rect_size = available_rect.size();
                let texture_size = match image {
                    LoadedImage::Raster { texture, .. } => texture.size_vec2(),
                    LoadedImage::Svg { texture, .. } => texture.size_vec2(),
                };
                let scaled_size = texture_size * self.scale;
                let pos = available_rect.min + (rect_size - scaled_size) * 0.5 + self.pan_offset;
                let rect = Rect::from_min_size(pos, scaled_size);
                ui.put(
                    rect,
                    egui::Image::new(match image {
                        LoadedImage::Raster { texture, .. } => texture,
                        LoadedImage::Svg { texture, .. } => texture,
                    })
                    .fit_to_exact_size(scaled_size),
                );

                // キー入力処理
                if ui.input(|i| i.key_pressed(Key::ArrowRight)) {
                    self.load_adjacent_image(ctx, true);
                } else if ui.input(|i| i.key_pressed(Key::ArrowLeft)) {
                    self.load_adjacent_image(ctx, false);
                } else if ui.input(|i| i.key_pressed(Key::F)) {
                    // Fキー：位置リセット＆フィットウィンドウ表示
                    self.pan_offset = Vec2::ZERO;
                    self.fit_to_screen(ctx);
                } else if ui.input(|i| i.key_pressed(Key::Num0)) {
                    // 0キー：位置リセット＆100%表示（scale = 1.0）
                    self.pan_offset = Vec2::ZERO;
                    self.scale = 1.0;
                } else if ui.input(|i| i.key_pressed(Key::O)) {
                    if let Some(file_path) = rfd::FileDialog::new()
                        .add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp", "svg"])
                        .pick_file()
                    {
                        self.load_image(&file_path, ctx);
                        self.update_image_list(&file_path);
                    }
                } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }

                let old_scale = self.scale;

                if ui.input(|i| i.key_pressed(Key::PlusEquals)) {
                    self.scale = (self.scale * 1.1).clamp(0.1, 10.0);

                    // 画面中央を基準に拡大
                    let image_center = available_rect.center().to_vec2();
                    let image_pos = available_rect.min.to_vec2() + (available_rect.size() - scaled_size) * 0.5 + self.pan_offset;
                    let offset_from_center = image_pos - image_center;
                    self.pan_offset = image_center + offset_from_center * (self.scale / old_scale) - available_rect.min.to_vec2() - (available_rect.size() - scaled_size * (self.scale / old_scale)) * 0.5;
                } else if ui.input(|i| i.key_pressed(Key::Minus)) {
                    self.scale = (self.scale / 1.1).clamp(0.1, 10.0);

                    // 画面中央を基準に縮小
                    let image_center = available_rect.center().to_vec2();
                    let image_pos = available_rect.min.to_vec2() + (available_rect.size() - scaled_size) * 0.5 + self.pan_offset;
                    let offset_from_center = image_pos - image_center;
                    self.pan_offset = image_center + offset_from_center * (self.scale / old_scale) - available_rect.min.to_vec2() - (available_rect.size() - scaled_size * (self.scale / old_scale)) * 0.5;
                }

                let response = ui.interact(
                    available_rect,
                    ui.id().with("drag_area"),
                    egui::Sense::click_and_drag(),
                );

                if response.dragged() && !ui.input(|i| i.pointer.secondary_down()) {
                    self.pan_offset += response.drag_delta();
                }

                let wheel_delta = ui.input(|i| i.scroll_delta.y);
                if wheel_delta != 0.0 {
                    let zoom_factor = 1.0 + wheel_delta * self.config.wheel_zoom_factor;
                    let new_scale = (self.scale * zoom_factor).clamp(0.1, 10.0);
                    self.scale = new_scale;

                    // マウスカーソル位置を基準に拡大縮小
                    if let Some(cursor_pos) = response.hover_pos() {
                        let cursor_pos = cursor_pos.to_vec2();
                        let image_pos = available_rect.min.to_vec2() + (available_rect.size() - scaled_size) * 0.5 + self.pan_offset;
                        
                        // カーソルから画像の相対位置を計算
                        let rel_pos = (cursor_pos - image_pos) / old_scale;
                        
                        // 新しい画像位置を計算
                        let new_image_pos = cursor_pos - rel_pos * self.scale;
                        self.pan_offset = new_image_pos - available_rect.min.to_vec2() - (available_rect.size() - scaled_size * (self.scale / old_scale)) * 0.5;
                    }
                }

                // マウスジェスチャーの更新と判定
                let mouse_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(Pos2::ZERO);
                let right_button = ui.input(|i| i.pointer.secondary_down());
                
                // updateの戻り値でアクションを受け取る
                if let Some(action) = self.mouse_gesture.update(mouse_pos.to_vec2(), right_button) {
                    match action.as_str() {
                        "<<<" => self.load_adjacent_image(ctx, false),
                        ">>>" => self.load_adjacent_image(ctx, true),
                        _ => {}
                    }
                }

                // マウスジェスチャーの描画
                if self.mouse_gesture.is_active {
                    self.mouse_gesture.draw(ui, available_rect.center());
                }
            }
        });

        // タイトルバーに、現在の拡大率とファイルパスを表示
        if let Some(image) = &self.current_image {
            let path_str = match image {
                LoadedImage::Raster { path, .. } => path.to_string_lossy(),
                LoadedImage::Svg { path, .. } => path.to_string_lossy(),
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                format!("MSBT-yuina - {}% - {}", (self.scale * 100.0) as i32, path_str)
            ));
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
