#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Vec2};
use egui::IconData;
use std::path::{Path, PathBuf};
use std::fs;
use log::{error, info, LevelFilter};
use std::io::Cursor;
use image::io::Reader as ImageReader;
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
};
use std::panic;
use ico;

fn init_logging() -> Result<(), Box<dyn std::error::Error>> {
    // 実行ファイルのパスを取得
    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path.parent().ok_or("Failed to get executable directory")?;
    
    // 実行ファイルの名前（拡張子なし）を取得してログファイル名を作成
    let exe_name = exe_path
        .file_stem()
        .ok_or("Failed to get executable name")?
        .to_string_lossy();
    let log_path = exe_dir.join(format!("{}.log", exe_name));

    // ログ設定
    let file_appender = FileAppender::builder()
        .encoder(Box::new(PatternEncoder::new("{d(%Y-%m-%d %H:%M:%S)} - {l} - {m}\n")))
        .build(log_path)?;

    let config = Config::builder()
        .appender(Appender::builder().build("file", Box::new(file_appender)))
        .build(Root::builder()
            .appender("file")
            .build(LevelFilter::Info))?;

    log4rs::init_config(config)?;
    Ok(())
}

fn main() -> eframe::Result<()> {
    // パニック時のログ出力を設定
    panic::set_hook(Box::new(|panic_info| {
        error!("アプリケーションがパニックで終了: {}", panic_info);
    }));

    // ログ初期化
    if let Err(e) = init_logging() {
        eprintln!("ログの初期化に失敗: {}", e);
        return Ok(());
    }

    info!("アプリケーション起動開始");

    let options = match create_app_options() {
        Ok(opt) => opt,
        Err(e) => {
            error!("アプリケーション設定の作成に失敗: {}", e);
            return Ok(());
        }
    };

    info!("アプリケーション設定の作成完了");

    match eframe::run_native(
        "Simple Image Viewer",
        options,
        Box::new(|cc| {
            info!("アプリケーションコンテキストの作成開始");
            Box::new(ImageViewer::default())
        }),
    ) {
        Ok(_) => {
            info!("アプリケーション正常終了");
            Ok(())
        }
        Err(e) => {
            error!("アプリケーション異常終了: {}", e);
            Err(e)
        }
    }
}

fn create_app_options() -> Result<eframe::NativeOptions, Box<dyn std::error::Error>> {
    info!("アプリケーション設定の作成開始");
    
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([200.0, 200.0])
            .with_drag_and_drop(true)
            .with_title("Simple Image Viewer")
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

struct ImageViewer {
    current_image: Option<LoadedImage>,
    current_path: Option<PathBuf>,
    initial_path: Option<PathBuf>,
    scale: f32,
    position: Vec2,
    image_list: Vec<PathBuf>,
    current_index: usize,
    dropped_files: Vec<PathBuf>,
    // UI状態管理用フィールド
    dragging: bool,
    drag_start: Option<Pos2>,
    last_cursor_pos: Option<Pos2>,
    // 画像サイズと表示領域
    image_size: Option<[u32; 2]>,
    available_size: Option<Vec2>,
    // 操作フラグ
    should_load_next: bool,
    should_load_prev: bool,
    open_file_dialog: bool,
}

struct LoadedImage {
    texture: egui::TextureHandle,
    path: PathBuf,
}

impl Default for ImageViewer {
    fn default() -> Self {
        info!("ImageViewerの初期化開始");
        let initial_path = std::env::args().nth(1).map(PathBuf::from);
        if let Some(ref path) = initial_path {
            info!("初期画像パス: {:?}", path);
        }

        let viewer = Self {
            current_image: None,
            current_path: None,
            initial_path,
            scale: 1.0,
            position: Vec2::ZERO,
            image_list: Vec::new(),
            current_index: 0,
            dropped_files: Vec::new(),
            // UI状態の初期化
            dragging: false,
            drag_start: None,
            last_cursor_pos: None,
            // サイズ情報の初期化
            image_size: None,
            available_size: None,
            // 操作フラグの初期化
            should_load_next: false,
            should_load_prev: false,
            open_file_dialog: false,
        };
        info!("ImageViewerの初期化完了");
        viewer
    }
}

impl ImageViewer {
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
                                        ["jpg", "jpeg", "png", "gif", "webp", "bmp"].contains(&ext.as_str())
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
                    self.image_list = files;
                    self.current_index = self.image_list.iter().position(|p| p == current_path).unwrap_or(0);
                    info!("ディレクトリの読み込みに成功しました: {:?}", parent);
                }
                Err(e) => {
                    error!("ディレクトリの読み込みに失敗しました: {:?} - エラー: {}", parent, e);
                }
            }
        }
    }

    fn update_scale(&mut self, ctx: &egui::Context) {
        if let (Some(available_size), Some(image_size)) = (self.available_size, self.image_size) {
            let image_size = Vec2::new(image_size[0] as f32, image_size[1] as f32);
            let scale_x = available_size.x / image_size.x;
            let scale_y = available_size.y / image_size.y;
            self.scale = scale_x.min(scale_y);
            self.position = Vec2::ZERO;
        }
    }

    fn load_image(&mut self, path: &Path, ctx: &egui::Context) {
        info!("画像を読み込もうとしています: {:?}", path);
        
        match fs::read(path) {
            Ok(buffer) => {
                info!("ファイルを読み込みました: {} bytes", buffer.len());
                
                // メモリ上のバッファから画像を読み込む
                match ImageReader::new(Cursor::new(buffer))
                    .with_guessed_format()
                    .map_err(|e| error!("フォーマットの推測に失敗: {}", e))
                    .and_then(|reader| reader.decode().map_err(|e| error!("デコードに失敗: {}", e)))
                {
                    Ok(img) => {
                        let size = [img.width(), img.height()];
                        let image_buffer = img.to_rgba8();
                        
                        // テクスチャを作成
                        let texture = ctx.load_texture(
                            "current_image",
                            egui::ColorImage::from_rgba_unmultiplied(
                                [size[0] as _, size[1] as _],
                                &image_buffer,
                            ),
                            Default::default(),
                        );
                        
                        self.current_image = Some(LoadedImage {
                            texture,
                            path: path.to_path_buf(),
                        });
                        self.current_path = Some(path.to_path_buf());
                        self.image_size = Some(size);
                        self.update_image_list(path);
                        info!("画像の読み込みに成功: {}x{}", size[0], size[1]);
                    }
                    Err(_) => {
                        error!("画像のデコードに失敗しました");
                        self.current_image = None;
                        self.current_path = None;
                        self.image_size = None;
                    }
                }
            }
            Err(e) => {
                error!("ファイルの読み込みに失敗しました: {}", e);
                self.current_image = None;
                self.current_path = None;
                self.image_size = None;
            }
        }
    }

    fn load_adjacent_image(&mut self, ctx: &egui::Context, next: bool) {
        if let Some(index) = self.current_index.checked_sub(1) {
            let new_index = if next {
                (index + 1) % self.image_list.len()
            } else {
                (index + self.image_list.len() - 1) % self.image_list.len()
            };
            let path = self.image_list[new_index].clone();
            self.load_image(&path, ctx);
        }
    }

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
                let rect = Rect::from_min_size(
                    rect.min + Vec2::new(x * size, y * size),
                    Vec2::splat(size),
                );
                painter.rect_filled(rect, 0.0, color);
                x += 1.0;
            }
            y += 1.0;
        }
    }

    fn handle_input(&mut self, ui: &egui::Ui) {
        if ui.input(|i| i.key_pressed(egui::Key::PlusEquals)) {
            self.scale *= 1.1;
            self.scale = self.scale.min(10.0);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Minus)) {
            self.scale *= 0.9;
            self.scale = self.scale.max(0.01);
        }
    }

    fn handle_keyboard_input(&mut self, ui: &egui::Ui) {
        // ズーム
        if ui.input(|i| i.key_pressed(egui::Key::PlusEquals)) {
            self.scale *= 1.1;
            self.scale = self.scale.min(10.0);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Minus)) {
            self.scale *= 0.9;
            self.scale = self.scale.max(0.1);
        }

        // 画像の切り替え
        if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) || ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            if let Some(current_path) = &self.current_path {
                if let Some(parent) = current_path.parent() {
                    if let Ok(entries) = fs::read_dir(parent) {
                        // 画像ファイルのみをフィルタリング
                        let mut image_files: Vec<_> = entries
                            .filter_map(|e| e.ok())
                            .map(|e| e.path())
                            .filter(|p| {
                                if let Some(ext) = p.extension() {
                                    let ext = ext.to_string_lossy().to_lowercase();
                                    return ext == "png" || ext == "jpg" || ext == "jpeg" || ext == "gif" || ext == "bmp";
                                }
                                false
                            })
                            .collect();

                        // ファイル名でソート
                        image_files.sort();

                        // 現在の画像のインデックスを探す
                        if let Some(current_index) = image_files.iter().position(|p| p == current_path) {
                            let new_index = if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                                if current_index == 0 {
                                    image_files.len() - 1
                                } else {
                                    current_index - 1
                                }
                            } else {
                                if current_index == image_files.len() - 1 {
                                    0
                                } else {
                                    current_index + 1
                                }
                            };

                            // 新しい画像を読み込む
                            if let Some(new_path) = image_files.get(new_index) {
                                info!("画像を読み込もうとしています: {:?}", new_path);
                                if let Ok(file_data) = fs::read(new_path) {
                                    info!("ファイルを読み込みました: {} bytes", file_data.len());
                                    if let Ok(img) = image::load_from_memory(&file_data) {
                                        let size = [img.width(), img.height()];
                                        let image_buffer = img.to_rgba8();
                                        let pixels = image_buffer.as_flat_samples();
                                        info!("画像の読み込みに成功: {}x{}", size[0], size[1]);

                                        self.current_image = Some(LoadedImage {
                                            texture: ui.ctx().load_texture(
                                                "image",
                                                egui::ColorImage::from_rgba_unmultiplied(
                                                    [size[0] as usize, size[1] as usize],
                                                    pixels.as_slice()
                                                ),
                                                egui::TextureOptions::default(),
                                            ),
                                            path: new_path.clone(),
                                        });
                                        self.current_path = Some(new_path.clone());
                                        self.image_size = Some(size);
                                        self.fit_to_screen(ui.ctx());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn fit_to_screen(&mut self, ctx: &egui::Context) {
        if let Some(image) = &self.current_image {
            let available_size = ctx.available_rect().size();
            let texture_size = image.texture.size();
            self.image_size = Some([texture_size[0] as u32, texture_size[1] as u32]);
            let image_size = Vec2::new(
                texture_size[0] as f32,
                texture_size[1] as f32,
            );
            let scale_x = available_size.x / image_size.x;
            let scale_y = available_size.y / image_size.y;
            self.scale = scale_x.min(scale_y);
            self.position = Vec2::ZERO;
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Load initial image if provided
        if let Some(path) = self.initial_path.take() {
            if let Ok(file_data) = fs::read(&path) {
                info!("ファイルを読み込みました: {} bytes", file_data.len());
                if let Ok(img) = image::load_from_memory(&file_data) {
                    let size = [img.width(), img.height()];
                    let image_buffer = img.to_rgba8();
                    let pixels = image_buffer.as_flat_samples();
                    info!("画像の読み込みに成功: {}x{}", size[0], size[1]);

                    self.current_image = Some(LoadedImage {
                        texture: ctx.load_texture(
                            "image",
                            egui::ColorImage::from_rgba_unmultiplied(
                                [size[0] as usize, size[1] as usize],
                                pixels.as_slice()
                            ),
                            egui::TextureOptions::default(),
                        ),
                        path: path.clone(),
                    });
                    self.current_path = Some(path);
                    self.image_size = Some(size);
                    self.fit_to_screen(ctx);
                }
            }
        }

        // Handle keyboard input
        egui::CentralPanel::default().show(ctx, |ui| {
            self.handle_keyboard_input(ui);
        });

        // タイトルを更新
        if let Some(image) = &self.current_image {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                format!("Simple Image Viewer - {}% - {}", (self.scale * 100.0) as i32, image.path.display())
            ));
        }

        // Draw image
        egui::CentralPanel::default().show(ctx, |ui| {
            // Draw checker background
            self.draw_checker_background(ui);

            // Draw image
            if let Some(image) = &self.current_image {
                let rect = ui.max_rect();
                let image_size = Vec2::new(
                    image.texture.size()[0] as f32,
                    image.texture.size()[1] as f32,
                );

                let scaled_size = image_size * self.scale;
                let min_pos = rect.center() - scaled_size * 0.5 + self.position;

                ui.put(
                    rect,
                    egui::Image::new(&image.texture)
                        .min_size(scaled_size)
                        .max_size(scaled_size)
                        .translate(min_pos - rect.min.to_vec2()),
                );

                // Handle drag
                if ui.rect_contains_pointer(rect) {
                    if ui.input(|i| i.pointer.secondary_down()) {
                        if let Some(delta) = ui.input(|i| i.pointer.delta()) {
                            self.position += delta;
                        }
                    }

                    // Handle zoom
                    if let Some(hover_pos) = ui.ctx().pointer_hover_pos() {
                        ui.input(|i| {
                            let zoom_delta = i.scroll_delta.y / 1000.0;
                            if zoom_delta != 0.0 {
                                let old_scale = self.scale;
                                self.scale *= 1.0 + zoom_delta;
                                self.scale = self.scale.clamp(0.1, 10.0);

                                // Adjust position to zoom towards cursor
                                let scale_delta = self.scale - old_scale;
                                let rect_center = rect.center();
                                let cursor_offset = hover_pos - rect_center;
                                let size_delta = image_size * scale_delta;
                                self.position -= size_delta * 0.5;
                                self.position -= cursor_offset * (scale_delta / old_scale);

                                // タイトルを更新
                                ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                                    format!("Simple Image Viewer - {}% - {}", (self.scale * 100.0) as i32, image.path.display())
                                ));
                            }
                        });
                    }
                }
            }
        });
    }
}

impl eframe::App for ImageViewer {}

fn create_fallback_icon() -> IconData {
    IconData {
        rgba: vec![0, 0, 0, 0],
        width: 1,
        height: 1,
    }
}

fn load_icon() -> IconData {
    info!("アイコンの読み込み開始");

    // アイコンの読み込みを試みる
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

        // 32x32のアイコンを探す（なければ最も近いサイズを使用）
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

        // IconImageをVec<u8>に変換
        let rgba: Vec<u8> = icon_image.rgba_data().to_vec();

        info!("アイコンの読み込み完了: {}x{} pixels", width, height);

        Ok(IconData {
            rgba,
            width,
            height,
        })
    }();

    match icon_result {
        Ok(icon) => icon,
        Err(e) => {
            error!("アイコンの読み込みに失敗: {}", e);
            create_fallback_icon()
        }
    }
}
